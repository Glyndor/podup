//! Rootless-Podman caveat warnings: fields compose accepts that rootless
//! Podman cannot honour (or fails on). Pure helper so it can be unit-tested
//! without a live engine.
//!
//! The cgroup-dependent limits (`oom_kill_disable`, `mem_swappiness`,
//! `cpu_rt_period`, `cpu_rt_runtime` and `blkio_config`) are not listed
//! here: whether the host honours them depends on the cgroup layout the
//! daemon reports, and on cgroups v1 they are applied. They are decided
//! where that is known, in `super::super::host_info`.

use crate::compose::types::Service;

/// Compose fields Podman accepts but cannot honor (or that fail) under
/// rootless Podman. Returns advisory messages; pure so it can be
/// unit-tested. The wording mirrors podman-run(1) so operators are not misled
/// into assuming a no-op limit took effect.
pub(in crate::backends::engine::workload::container) fn rootless_caveat_warnings(
	name: &str,
	service: &Service,
) -> Vec<String> {
	let mut out = Vec::new();
	if service.privileged == Some(true) {
		out.push(format!(
			"service \"{name}\": privileged has reduced effect under rootless Podman: a \
			container cannot gain more privileges than the user that launched it"
		));
	}
	if !service.links.is_empty() {
		out.push(format!(
			"service \"{name}\": links has no effect under rootless Podman networking; put the \
			services on a shared network and reach them by service name instead"
		));
	}
	if !service.external_links.is_empty() {
		out.push(format!(
			"service \"{name}\": external_links has no effect under rootless Podman networking; \
			attach the target container to a shared network and reach it by service name instead"
		));
	}
	out
}

#[cfg(test)]
#[path = "rootless_tests.rs"]
mod tests;
