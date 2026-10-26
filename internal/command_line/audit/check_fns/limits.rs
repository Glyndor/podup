use podup::compose::types::{ComposeFile, Service};

use super::{finding, Finding};

// Shared with the engine's `build_resource_limits` (#1894): the audit reads
// the same memory cap the engine forwards into the OCI spec, applying the
// top-level/deploy precedence `build_resource_limits` does.
use podup::effective_memory_limit;

/// `pids_limit` unset, no ceiling on the number of processes the container
/// can fork, so a runaway loop can starve the host out of PIDs.
pub fn check_no_pids_limit(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	if service.pids_limit.is_none() {
		vec![finding(
			name,
			"no_pids_limit",
			"pids_limit is not set: a fork bomb can exhaust the host's process table",
		)]
	} else {
		Vec::new()
	}
}

/// Neither `mem_limit` nor `deploy.resources.limits.memory` set, no upper
/// bound on memory. A misbehaving service can OOM the host.
///
/// Reads both fields through the same `parse_memory` the engine uses to
/// build `LinuxResources` (`#1743`): a value like `mem_limit: not-a-size`
/// keeps the compose field non-empty, so an `.is_none()` audit reads it
/// as a limit, but `parse_memory` returns `None` and the runtime applies
/// no limit at all. The audit must agree with what the engine will build.
///
/// The shared [`podup::effective_memory_limit`] helper applies the same
/// top-first/deploy-fill precedence `build_resource_limits` does, so a
/// top-level `mem_limit: 256m` is treated as the effective cap even when
/// `deploy.resources.limits.memory` carries a different value (the deploy
/// block is ignored once the top level set one). Reading both fields
/// independently here would silently take the larger one and disagree
/// with what the engine forwards to libpod (`#1894`).
///
/// The shared helper returns what the engine forwards verbatim,
/// including `Some(-1)` for `mem_limit: "-1"`. Podman interprets `-1`
/// as "no cap" and the audit agrees: the operator did not set a
/// memory limit, so this check fires the same way it does for an
/// absent key (`#1894`).
pub fn check_no_memory_limit(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	if effective_memory_limit(service).is_none_or(|v| v < 0) {
		vec![finding(
			name,
			"no_memory_limit",
			"neither mem_limit nor deploy.resources.limits.memory is parseable: a leak can OOM the host",
		)]
	} else {
		Vec::new()
	}
}
