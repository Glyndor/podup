//! The five runtime-hardening checks from #1894:
//! `no_restart_policy`, `no_init`, `no_health_action`, `swap_unbounded`,
//! `no_cpu_limit`. Split into their own file because the registry already
//! counts `check_fns.rs` at 329 code lines, and adding five more would push
//! it past the 500-line limit the rest of the audit honours. Wired the
//! way `check_sensitive_bind.rs` is: a private helper per check, a public
//! `check_*` function the registry points at, and `super::Finding` +
//! `super::check_fns::finding` reused for the row shape so every entry in
//! `CHECK_REGISTRY` looks the same to the listing renderer and the audit
//! dispatch path.

use podup::compose::types::{ComposeFile, Service};
use podup::size;

use super::super::Finding;
use super::check_fns::finding;

/// The memory limit the engine will apply, computed the same way
/// [`super::check_fns::check_no_memory_limit`] computes it: read
/// `mem_limit` and `deploy.resources.limits.memory` through the engine's
/// own `size::parse_memory` so an unparseable value counts as no limit
/// (#1743). Reused by [`check_swap_unbounded`] so the swap check and the
/// memory check agree on what "the memory limit in effect" is, instead of
/// drifting on a re-implementation.
///
/// Returns `None` when neither field parses to a positive byte count;
/// `Some(-1)` is the engine's "unlimited" sentinel for `parse_memory`,
/// which the swap check treats as no memory limit in effect (Podman
/// refuses to apply a memory cap when the field is `-1`, so the swap
/// comparison is meaningless).
fn memory_limit_in_effect(service: &Service) -> Option<i64> {
	let mem_top = service.mem_limit.as_deref().and_then(size::parse_memory);
	let deploy_limit = service
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.limits.as_ref())
		.and_then(|l| l.memory.as_deref().and_then(size::parse_memory));
	match (mem_top, deploy_limit) {
		(Some(a), Some(b)) => Some(a.max(b)),
		(Some(a), None) | (None, Some(a)) => Some(a),
		(None, None) => None,
	}
}

/// Same shape for the CPU limit: read `cpus` (service level) and
/// `deploy.resources.limits.cpus` through `size::parse_cpus`, and use
/// `cpu_quota` if set. Returns the resolved CPU limit in nano-CPUs
/// when `cpus:` parsed, or the sentinel `-1` when `cpu_quota:` is set
/// without a parsed `cpus:`, otherwise `None`. The engine at
/// `internal/engine/container_config/resources.rs::build_resource_limits`
/// treats any of the three as a CPU limit, so the audit must agree.
///
/// `cpu_quota` is a CFS hard cap in microseconds over `cpu_period`; the
/// engine does not convert it to nano-CPUs (it stays as a quota), so the
/// audit only needs to detect its presence, not its magnitude.
fn cpu_limit_in_effect(service: &Service) -> Option<i64> {
	let top = service.cpus.as_deref().and_then(size::parse_cpus);
	let deploy = service
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.limits.as_ref())
		.and_then(|l| l.cpus.as_deref().and_then(size::parse_cpus));
	let nanos = match (top, deploy) {
		(Some(a), Some(b)) => Some(a.max(b)),
		(Some(a), None) | (None, Some(a)) => Some(a),
		(None, None) => None,
	};
	if nanos.is_some() {
		return nanos;
	}
	if service.cpu_quota.is_some() {
		return Some(-1);
	}
	None
}

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
/// would silently let a sick container stay sick.
pub fn check_no_health_action(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	let Some(hc) = &service.healthcheck else {
		return Vec::new();
	};
	if hc.is_disabled() {
		return Vec::new();
	}
	// `Err` here means a typo or non-string value the engine will reject
	// at validation; treat it as "the operator typed something" rather
	// than silently firing.
	if hc.podman_on_failure().is_ok_and(|opt| opt.is_some()) {
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
/// The audit reuses [`memory_limit_in_effect`] so the swap check and
/// `no_memory_limit` agree on which value they call "the memory limit";
/// without that sharing, the swap check could compare against a string
/// the engine already rejected.
///
/// No memory limit in effect -> silent: that case is `no_memory_limit`'s
/// finding, not this one's. Reporting both would double-count the same
/// risk.
pub fn check_swap_unbounded(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	let Some(limit) = memory_limit_in_effect(service) else {
		return Vec::new();
	};
	// `parse_memory("-1")` returns `Some(-1)`; treat it the same as
	// "absent" because Podman interprets `-1` as unlimited swap, not as a
	// bounded swap limit.
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
/// nor `cpu_quota:` gives the container a hard cap. The engine at
/// `internal/engine/container_config/resources.rs::build_resource_limits`
/// converts `cpus:` to a CFS quota and forwards `cpu_quota:` verbatim;
/// either is a limit. An unparseable `cpus:` value the engine silently
/// drops is reported the same way: the audit cannot tell the engine
/// dropped it, so it reports the unresolved value as a missing limit.
///
/// `cpu_shares` and `cpuset` are deliberately NOT counted: `cpu_shares`
/// is a relative weight under contention (1024 is the default), not a
/// hard cap, and `cpuset` is a placement constraint, not a quota.
pub fn check_no_cpu_limit(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	if cpu_limit_in_effect(service).is_none() {
		return vec![finding(
			name,
			"no_cpu_limit",
			"neither cpus nor deploy.resources.limits.cpus nor cpu_quota gives a limit: one service can take every core of the host",
		)];
	}
	Vec::new()
}
