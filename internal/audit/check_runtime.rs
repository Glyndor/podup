//! The five runtime-hardening checks from #1894:
//! `no_restart_policy`, `no_init`, `no_health_action`, `swap_unbounded`,
//! `no_cpu_limit`. Split into their own file to keep `check_fns.rs`
//! well under the 500-line limit the rest of the audit honours; adding
//! five more checks there would otherwise push it past the ceiling.
//! Wired the way `check_sensitive_bind.rs` is: a private helper per
//! check, a public `check_*` function the registry points at, and
//! `super::Finding` + `super::check_fns::finding` reused for the row
//! shape so every entry in `CHECK_REGISTRY` looks the same to the
//! listing renderer and the audit dispatch path.

use podup::compose::types::{ComposeFile, Service};
use podup::size;

use super::super::Finding;
use super::check_fns::finding;
// Shared with the engine's `build_resource_limits` (#1894): the audit
// reads the same value the engine forwards into the OCI spec, applying
// the top-level/deploy precedence `build_resource_limits` does. The CPU
// helper below does the same for CPU; `cpuset:` is audit-only and
// lives in [`cpu_limit_in_effect`] because the engine forwards it as a
// placement constraint separate from the quota.
//
// The engine helpers return what the engine forwards verbatim, including
// `cpu_quota: -1` and `mem_limit: "-1"`. The audit applies the
// "zero-or-below is not a limit" / "-1 is not a cap" filters on top,
// in [`cpu_limit_in_effect`] and [`check_no_memory_limit`] /
// [`check_swap_unbounded`] below.
use podup::effective_cpu_quota;
use podup::effective_memory_limit;

// ---------------------------------------------------------------------------
// Audit-side wrappers around the engine's effective-limit helpers
// ---------------------------------------------------------------------------

/// "Is there a CPU cap in effect?" for the audit: a positive
/// `cpu_quota:`, OR a positive `cpus:` (top-level first, then
/// `deploy.resources.limits.cpus:`), OR a non-empty `cpuset:`. The first
/// two come from the shared [`podup::effective_cpu_quota`] helper so
/// the audit and `build_resource_limits` agree on what the OCI spec
/// will carry; the `cpuset:` clause is audit-only because the engine
/// forwards `cpuset:` as a placement constraint separate from the
/// quota (`#1894`).
///
/// The shared helper returns `cpu_quota` verbatim, including `-1`
/// (Docker's "unlimited" sentinel) and `0` (a quota with no CPU time).
/// Both count as "no limit" for the audit: an operator who wrote
/// `cpu_quota: -1` did not set a cap, and `0` is not a bound either.
/// This wrapper applies that filter.
///
/// `cpuset:` pins the container to named cores, which bounds how many
/// cores it can take even when no quota is in effect; an audit that
/// ignored it would fire on a `cpuset: "0-1"` declaration Podman will
/// honor as a single-core pin. An empty or whitespace-only `cpuset:` is
/// treated as not set (it carries no placement constraint).
fn cpu_limit_in_effect(service: &Service) -> bool {
	if effective_cpu_quota(service).is_some_and(|q| q > 0) {
		return true;
	}
	service
		.cpuset
		.as_deref()
		.is_some_and(|s| !s.trim().is_empty())
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

/// `restart:` and `deploy.restart_policy:` both reach the same engine
/// knob (`internal/engine/container_config/mod.rs::build_restart_policy`):
/// top-level `restart:` wins, otherwise `deploy.restart_policy.condition`
/// applies with the engine's `any` -> `always` mapping. The audit must
/// agree: a deliberate `restart: "no"` is a choice (the operator decided
/// they don't want restarts) and must stay silent, while a missing
/// `restart:` AND a missing `deploy.restart_policy:` is the unguarded
/// case the check fires on.
pub fn check_no_restart_policy(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	if service.restart.is_some() {
		return Vec::new();
	}
	if service
		.deploy
		.as_ref()
		.and_then(|d| d.restart_policy.as_ref())
		.is_some()
	{
		return Vec::new();
	}
	vec![finding(
		name,
		"no_restart_policy",
		"neither restart nor deploy.restart_policy is set: a process that exits stays exited until someone redeploys",
	)]
}

/// `init:` not `true`: PID 1 is the application process itself, so
/// orphaned children become zombies the container will not reap, and a
/// `SIGTERM` sent to PID 1 may wait out the whole stop timeout before the
/// kernel escalates to `SIGKILL`. The compose default is `false`, so an
/// absent key and an explicit `false` both leave the gate unguarded.
pub fn check_no_init(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	if service.init == Some(true) {
		return Vec::new();
	}
	vec![finding(
		name,
		"no_init",
		"init is not true: PID 1 is the app, orphans become zombies and SIGTERM may wait out the whole stop timeout",
	)]
}

/// A compose `healthcheck:` that is not disabled and has no
/// `x-podman-on-failure` action: the spec's health check detects a sick
/// container but takes no action on it. The Podman extension is the
/// documented way to wire one of `none | kill | restart | stop` to the
/// unhealthy transition.
///
/// The audit uses the same `is_disabled` accessor the engine uses
/// (`disable: true` OR `test: ["NONE"]`), and the same `podman_on_failure`
/// accessor (`x-podman-on-failure: <value>`). An invalid value is
/// `validate`'s job, not the audit's: an `Err` from `podman_on_failure`
/// is treated here as "an action is set" (the operator typed something;
/// a typo will fail validation upstream) rather than "no action", which
/// would silently let a sick container stay sick. `Ok(None)` is the only
/// case the check fires on.
pub fn check_no_health_action(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	let Some(hc) = &service.healthcheck else {
		return Vec::new();
	};
	if hc.is_disabled() {
		return Vec::new();
	}
	// `Err` here means a typo or non-string value the engine will reject
	// at validation; treat it as "the operator typed something" rather
	// than silently firing. `Ok(Some(_))` is the legitimate "an action is
	// set" case. Only `Ok(None)` (the key absent) leaves the check firing.
	if !matches!(hc.podman_on_failure(), Ok(None)) {
		return Vec::new();
	}
	vec![finding(
		name,
		"no_health_action",
		"healthcheck is set but x-podman-on-failure is not: an unhealthy container stays unhealthy and nothing acts on it",
	)]
}

/// A memory limit is in effect and `memswap_limit:` is either absent, is
/// `-1` (Podman's "unlimited swap"), or differs from the memory limit.
/// The engine at
/// `internal/engine/container_config/resources.rs::build_resource_limits`
/// forwards `memswap_limit` verbatim into `LinuxMemory.swap`, so an
/// absent value resolves to whatever Podman's default is (twice the
/// memory limit at the time of #1894's measurement); a `-1` explicitly
/// opts into unlimited swap; a value larger than the memory limit raises
/// the effective ceiling beyond `mem_limit`.
///
/// The audit reuses [`podup::effective_memory_limit`] so the swap check
/// and `no_memory_limit` agree on which value they call "the memory
/// limit". The helper applies the same top-first / deploy-fill
/// precedence the engine's `build_resource_limits` applies, so a
/// top-level `mem_limit: 256m` is treated as the cap even when
/// `deploy.resources.limits.memory: 512m` carries a larger value (the
/// deploy block is ignored once the top level set one). Without that
/// sharing, the swap check could compare against a value the engine
/// already rejected.
///
/// The shared helper forwards the engine's value verbatim, including
/// `mem_limit: "-1"`. The audit treats `Some(-1)` as "no memory limit"
/// here so this check and `no_memory_limit` agree that `mem_limit:
/// "-1"` is a missing cap, not a present one (`#1894`).
///
/// No memory limit in effect -> silent: that case is `no_memory_limit`'s
/// finding, not this one's. Reporting both would double-count the same
/// risk.
pub fn check_swap_unbounded(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	// `parse_memory("-1")` returns `Some(-1)`; Podman interprets that as
	// "no cap", and the audit agrees (see the shared helper's doc
	// comment in `internal/engine/container_config/resources.rs`).
	let Some(limit) = effective_memory_limit(service).filter(|&v| v >= 0) else {
		return Vec::new();
	};
	// Same `-1` filter for the swap side: `memswap_limit: "-1"` is the
	// "unlimited swap" sentinel, not a bounded swap limit.
	let swap = service
		.memswap_limit
		.as_deref()
		.and_then(size::parse_memory)
		.filter(|v| *v != -1);
	match swap {
		None => vec![finding(
			name,
			"swap_unbounded",
			"memswap_limit is absent or unlimited: the service can page to disk instead of hitting its memory limit",
		)],
		Some(v) if v != limit => vec![finding(
			name,
			"swap_unbounded",
			&format!(
				"memswap_limit ({v}) differs from the memory limit ({limit}): the service can page to disk instead of hitting its memory limit"
			),
		)],
		Some(_) => Vec::new(),
	}
}

/// No CPU limit in effect: neither `cpus:`, `deploy.resources.limits.cpus:`,
/// `cpu_quota:`, nor `cpuset:` gives the container a hard cap. The
/// engine at
/// `internal/engine/container_config/resources.rs::build_resource_limits`
/// converts `cpus:` to a CFS quota and forwards `cpu_quota:` verbatim;
/// either is a limit. An unparseable `cpus:` value the engine silently
/// drops is reported the same way: the audit cannot tell the engine
/// dropped it, so it reports the unresolved value as a missing limit.
///
/// A `cpu_quota:` of zero or below is rejected by the audit: the Docker
/// API treats `-1` as "unlimited" and Podman does the same, so the
/// operator's intent is not the limit they got. The shared
/// [`podup::effective_cpu_quota`] helper forwards the engine's value
/// verbatim; [`cpu_limit_in_effect`] applies the `<= 0` filter on top
/// of it.
///
/// `cpu_shares` is deliberately NOT counted: it is a relative weight
/// under contention (1024 is the default), not a hard cap. `cpuset:` IS
/// counted (see [`cpu_limit_in_effect`]): a non-empty `cpuset:` pins the
/// service to named cores, which bounds how many cores it can take.
pub fn check_no_cpu_limit(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	if !cpu_limit_in_effect(service) {
		return vec![finding(
			name,
			"no_cpu_limit",
			"neither cpus nor deploy.resources.limits.cpus nor cpu_quota nor cpuset gives a limit: one service can take every core of the host",
		)];
	}
	Vec::new()
}
