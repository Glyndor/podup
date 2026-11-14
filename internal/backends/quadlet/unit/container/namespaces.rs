//! Container namespaces stay independent even in a pod that shares only net.

use crate::compose::types::Service;

use super::{container_name, quote_podman_arg_value, Section, UnitContext};

/// Use CLI flags available at the Quadlet floor; PodmanArgs does not resolve
/// sibling unit references, so IPC/PID must name the exported container itself.
pub(super) fn emit_namespaces(service: &Service, ctx: &UnitContext<'_>, container: &mut Section) {
	for (flag, mode, service_reference) in [
		("ipc", &service.ipc, true),
		("pid", &service.pid, true),
		("uts", &service.uts, false),
		("cgroupns", &service.cgroup, false),
	] {
		let Some(mode) = mode.as_deref() else {
			continue;
		};
		let resolved;
		let mode = if let Some(target) = mode.strip_prefix("service:").filter(|_| service_reference)
		{
			// Validation rejects missing donors before any unit is written.
			let Some(donor) = ctx.services.get(target) else {
				continue;
			};
			resolved = format!("container:{}", container_name(ctx.project, target, donor));
			resolved.as_str()
		} else {
			mode
		};
		container.add(
			"PodmanArgs",
			format!("--{flag}={}", quote_podman_arg_value(mode)),
		);
	}
}
