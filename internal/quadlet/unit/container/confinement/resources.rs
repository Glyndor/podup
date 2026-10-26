//! The limits a container runs under: memory, CPU, pids and file descriptors.

use crate::compose::types::Service;

use super::super::super::pod_fields::{PodField, PodFields};
use super::super::super::shared_values::shm_size;
use super::super::{quote_podman_arg_value, Section};

/// Emit one `Ulimit=` line per `ulimits:` entry.
pub(in super::super) fn emit_ulimits(service: &Service, container: &mut Section) {
	for (name, limit) in &service.ulimits {
		let soft = limit.soft();
		let hard = limit.hard();
		let value = if soft == hard {
			format!("{name}={soft}")
		} else {
			format!("{name}={soft}:{hard}")
		};
		container.add("Ulimit", value);
	}
}

/// Emit `ShmSize=` plus the memory, CPU and pids limits of the service.
pub(in super::super) fn emit_limits(
	service: &Service,
	in_pod: bool,
	pod_fields: &PodFields,
	container: &mut Section,
) {
	if !(in_pod && pod_fields.owns(PodField::ShmSize)) {
		if let Some(shm) = shm_size(service) {
			container.add("ShmSize", shm.to_string());
		}
	}
	if let Some(mem) = &service.mem_limit {
		// `Memory=` is not a recognised [Container] Quadlet key (Quadlet would drop
		// the whole unit at daemon-reload), so route the limit through PodmanArgs=
		// as `--memory`, like the CPU limits. The value is validated as a size
		// before generation, so it is a well-formed limit here. The value is
		// still quoted (and `%`-doubled) so an interpolation site cannot smuggle
		// additional podman args into the same argv (#1734).
		container.add(
			"PodmanArgs",
			format!("--memory={}", quote_podman_arg_value(mem)),
		);
	}
	// CPU limits have no native [Container] Quadlet key (unlike Memory=/
	// PidsLimit=), so they go through PodmanArgs=. Quoted per #1734, same
	// reason as `mem_limit` above.
	// `cpus` falls back to the modern `deploy.resources.limits.cpus`.
	let deploy_cpus = service
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.limits.as_ref())
		.and_then(|l| l.cpus.as_deref());
	if let Some(c) = service.cpus.as_deref().or(deploy_cpus) {
		container.add(
			"PodmanArgs",
			format!("--cpus={}", quote_podman_arg_value(c)),
		);
	}
	if let Some(cs) = &service.cpuset {
		container.add(
			"PodmanArgs",
			format!("--cpuset-cpus={}", quote_podman_arg_value(cs)),
		);
	}
	if let Some(sh) = service.cpu_shares {
		container.add("PodmanArgs", format!("--cpu-shares={sh}"));
	}
	if let Some(q) = service.cpu_quota {
		container.add("PodmanArgs", format!("--cpu-quota={q}"));
	}
	if let Some(p) = service.cpu_period {
		container.add("PodmanArgs", format!("--cpu-period={p}"));
	}
	// `deploy.resources.limits.pids` is the modern equivalent of `pids_limit`.
	let deploy_pids = service
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.limits.as_ref())
		.and_then(|l| l.pids);
	if let Some(pids) = service.pids_limit {
		container.add("PidsLimit", pids.to_string());
	} else if let Some(pids) = deploy_pids {
		container.add("PidsLimit", pids.to_string());
	}
}

/// Emit the memory limit declared under `deploy.resources.limits.memory`, for a
/// service that declares no `mem_limit:` of its own.
pub(in super::super) fn emit_deploy_limits(service: &Service, container: &mut Section) {
	// `deploy.resources.limits.memory` is the modern equivalent of `mem_limit`.
	if service.mem_limit.is_none() {
		if let Some(mem) = service
			.deploy
			.as_ref()
			.and_then(|d| d.resources.as_ref())
			.and_then(|r| r.limits.as_ref())
			.and_then(|l| l.memory.as_ref())
		{
			container.add(
				"PodmanArgs",
				format!("--memory={}", quote_podman_arg_value(mem)),
			);
		}
	}
}
