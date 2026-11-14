//! The limits a container runs under: memory, CPU, pids and file descriptors.

use crate::compose::types::Service;
use crate::size;

use super::super::super::pod::{PodField, PodFields};
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
	if let Some(mem) = service.mem_limit.as_ref().filter(|m| !m.trim().is_empty()) {
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
	// Podman refuses `--cpus` next to `--cpu-quota` or `--cpu-period`. `up`
	// sends one CFS quota (`cpu_quota` first, else derived from `cpus`), so with
	// either of those set the same quota goes out as `--cpu-quota` instead.
	let cfs_explicit = service.cpu_quota.is_some() || service.cpu_period.is_some();
	if let Some(c) = service.cpus.as_deref().or(deploy_cpus) {
		if !cfs_explicit {
			container.add(
				"PodmanArgs",
				format!("--cpus={}", quote_podman_arg_value(c)),
			);
		}
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
	if let Some(q) = service.cpu_quota.or_else(|| {
		cfs_explicit
			.then(|| crate::engine::effective_cpu_quota(service))
			.flatten()
	}) {
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

fn deploy_memory_reservation(service: &Service) -> Option<&str> {
	service
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.reservations.as_ref())
		.and_then(|r| r.memory.as_deref())
}

/// Resolve the soft memory limit the engine forwards to the OCI spec.
///
/// The top-level value wins when the engine can read it; otherwise the
/// modern `deploy.resources.reservations.memory` value fills the gap.
fn readable_memory_reservation(service: &Service) -> Option<&str> {
	service
		.mem_reservation
		.as_deref()
		.filter(|value| size::parse_memory(value).is_some())
		.or_else(|| {
			deploy_memory_reservation(service).filter(|value| size::parse_memory(value).is_some())
		})
}

/// Render a memory amount in a form Podman's CLI accepts.
///
/// The engine parser accepts some spellings that Podman's CLI rejects, such
/// as surrounding whitespace or a leading decimal point. Byte counts preserve
/// the engine's value without duplicating Podman's size grammar.
fn podman_memory_value(value: &str) -> String {
	let trimmed = value.trim();
	if trimmed.chars().any(char::is_whitespace)
		|| trimmed.starts_with('+')
		|| trimmed.starts_with('-')
		|| trimmed.contains('.')
	{
		size::parse_memory(value)
			.map(|bytes| bytes.to_string())
			.unwrap_or_else(|| trimmed.to_string())
	} else {
		trimmed.to_string()
	}
}

/// Emit the memory and OOM controls that have no native Quadlet key.
///
/// Memory sizes are emitted only when the engine's parser can read them, so a
/// generated unit never fails later on a value that `up` would have omitted.
/// An unreadable value is reported instead of being silently dropped.
pub(in super::super) fn emit_memory_options(
	name: &str,
	service: &Service,
	container: &mut Section,
	warnings: &mut Vec<String>,
) {
	let reservation = readable_memory_reservation(service);
	// `-1` asks for no soft limit, which is what a container gets without the
	// flag, and Podman's CLI refuses a negative reservation.
	if let Some(value) = reservation.filter(|v| size::parse_memory(v) != Some(-1)) {
		let value = podman_memory_value(value);
		container.add(
			"PodmanArgs",
			format!("--memory-reservation={}", quote_podman_arg_value(&value)),
		);
	}
	if service
		.mem_reservation
		.as_deref()
		.is_some_and(|value| !value.trim().is_empty() && size::parse_memory(value).is_none())
	{
		warnings.push(format!(
			"{name}: mem_reservation cannot be read and is skipped"
		));
	}
	if reservation.is_none()
		&& deploy_memory_reservation(service)
			.is_some_and(|value| !value.trim().is_empty() && size::parse_memory(value).is_none())
	{
		warnings.push(format!(
			"{name}: deploy.resources.reservations.memory cannot be read and is skipped"
		));
	}

	// crun refuses a swap limit without a memory limit, as Docker does, so the
	// unit would never start; `up` fails on the same file when it creates the
	// container.
	let has_memory_limit = service
		.mem_limit
		.as_deref()
		.is_some_and(|m| !m.trim().is_empty())
		|| service
			.deploy
			.as_ref()
			.and_then(|d| d.resources.as_ref())
			.and_then(|r| r.limits.as_ref())
			.and_then(|l| l.memory.as_deref())
			.is_some_and(|m| !m.trim().is_empty());
	match service
		.memswap_limit
		.as_deref()
		.filter(|v| !v.trim().is_empty())
	{
		Some(value) if size::parse_memory(value).is_none() => warnings.push(format!(
			"{name}: memswap_limit cannot be read and is skipped"
		)),
		Some(_) if !has_memory_limit => warnings.push(format!(
			"{name}: memswap_limit needs mem_limit or deploy.resources.limits.memory; \
			 Podman refuses a swap limit without one, so it is skipped"
		)),
		Some(value) => container.add(
			"PodmanArgs",
			format!(
				"--memory-swap={}",
				quote_podman_arg_value(&podman_memory_value(value))
			),
		),
		None => {}
	}

	// Podman refuses both on cgroups v2 (`crun: cannot disable OOM killer with
	// cgroupv2`), which systemd hosts run, so a unit carrying them would not
	// start. Rootless `up` skips them with a warning for the same reason.
	let cgroups_v2 = "is skipped: Podman refuses it on cgroups v2, so the unit would not start";
	if service.mem_swappiness.is_some() {
		warnings.push(format!("{name}: mem_swappiness {cgroups_v2}"));
	}
	if service.oom_kill_disable == Some(true) {
		warnings.push(format!("{name}: oom_kill_disable {cgroups_v2}"));
	}
	if let Some(value) = service.oom_score_adj {
		let value = value.to_string();
		container.add(
			"PodmanArgs",
			format!("--oom-score-adj={}", quote_podman_arg_value(&value)),
		);
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
