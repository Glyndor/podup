//! Build the `.pod` Quadlet unit for a project.
//!
//! Emitted alongside the `.container` units when the compose file declares
//! `x-podman-pod: true`. The unit carries every port the pod publishes
//! (the union of every service's `ports:`), every declared network
//! (`Network=`) and the project ownership label; each `.container` unit
//! references this pod by `Pod=<stem>.pod` and drops its own `PublishPort=`
//! and `Network=` lines. The `.container` side of that contract is in
//! `super::container::container_unit`.
//!
//! Both `AddHost=` and `Label=` are routed through `PodmanArgs=` here: the
//! first appeared in 5.3.0 and the second in 5.6.0, but the supported floor
//! is 5.0. The same flags on the container side go the same way; see the
//! `emit_log_config` comment in `super::container` for the same reasoning.

use crate::compose::types::ComposeFile;
use crate::ports;

use super::{
	owner_marker, quote_podman_arg_value, render_publish_port, safe_unit_stem, unit_stem,
	QuadletUnit, Section,
};

/// Build the `.pod` unit for one project. The contents are a single
/// `[Pod]` section with `PodName=`, one `Network=` per declared network,
/// one `PublishPort=` per port (the union of every service's `ports:`),
/// and one `--add-host` flag per service. Each `.container` unit references
/// this pod by `Pod=<stem>.pod`; see [`super::container::container_unit`].
///
/// Returns `None` when the project has no pod-mode extension, so callers
/// can splice the unit into the output list without a conditional.
pub(crate) fn pod_unit(project: &str, file: &ComposeFile) -> Option<QuadletUnit> {
	if !file.podman_pod().unwrap_or(false) {
		return None;
	}

	let mut pod = Section::new("Pod");
	pod.add(
		"PodName",
		// The pod is named after the project so `podman pod` and the
		// generated `Pod=<stem>.pod` references line up with what the
		// live engine creates.
		project.to_string(),
	);

	// Every non-external network the file declares, plus the external ones by
	// their own name. Mirrors the live engine's `pod_networks` builder.
	for (key, config) in &file.networks {
		let external = config.as_ref().and_then(|c| c.external).unwrap_or(false);
		if external {
			let external_name = config
				.as_ref()
				.and_then(|c| c.name.clone())
				.unwrap_or_else(|| key.clone());
			pod.add("Network", external_name);
			continue;
		}
		// A declared network is backed by a generated `.network` unit, the same
		// reference a `.container` unit makes outside pod mode.
		pod.add("Network", format!("{}.network", unit_stem(project, key)));
	}

	// Union of every service's `ports:`. The live engine hands the same
	// union to `PodSpecGenerator.portmappings`, so what we write here must
	// match what `up` would create. Sorted through the same `parsed_ports`
	// shape the engine uses for its own hash so iteration order is stable
	// across runs (an unsorted loop would flip on HashMap iteration).
	let mut ports: Vec<ports::ParsedPort> = Vec::new();
	for service in file.services.values() {
		if let Ok(parsed) = ports::parse_ports(&service.ports) {
			ports.extend(parsed);
		}
	}
	ports.sort_by(|a, b| {
		(&a.host_ip, a.host_port, a.container_port, &a.protocol).cmp(&(
			&b.host_ip,
			b.host_port,
			b.container_port,
			&b.protocol,
		))
	});
	for p in &ports {
		pod.add("PublishPort", render_publish_port(p));
	}

	// One `<service>:127.0.0.1` per service, so a compose `db:5432` reference
	// resolves to the shared namespace the way it resolves on a project
	// network. The live engine builds the same map. `AddHost=` was added to
	// Quadlet in Podman 5.3.0, but the supported floor is 5.0, so route the
	// entry through `PodmanArgs=` as `--add-host`, like the container side.
	// Quoted per #1734: a hostile value carrying whitespace must not become
	// two argv elements.
	let mut hosts: Vec<String> = file
		.services
		.keys()
		.map(|s| format!("{s}:127.0.0.1"))
		.collect();
	hosts.sort();
	for entry in &hosts {
		pod.add(
			"PodmanArgs",
			format!("--add-host={}", quote_podman_arg_value(entry)),
		);
	}

	// Carry the project ownership label the live engine stamps onto pods, so
	// `down`/`down --remove-orphans` can find the unit by label. `Label=` on
	// `[Pod]` was added in Podman 5.6.0; the supported floor is 5.0, so the
	// label travels through `PodmanArgs=` as `--label=` for the same reason
	// the host entries above go through `--add-host=`. Quoted per #1734.
	pod.add(
		"PodmanArgs",
		format!(
			"--label={}",
			quote_podman_arg_value(&format!("podup.project={project}"))
		),
	);

	// Pin the pod's exit policy to `continue` so the unit Quadlet
	// generates agrees with what the live engine stamps on the pod it
	// creates. Quadlet hard-codes `--exit-policy stop` in the command line
	// it builds for `podman pod create`, and the native `ExitPolicy=` key
	// needs Podman 5.6 while the floor is 5.0. `PodmanArgs=` flags are
	// appended after it on that line, so this flag wins, and the generated
	// pod keeps running when its last service container exits. The live
	// engine sets the same value explicitly: the API path defaults to
	// `continue`, but that default comes from `containers.conf`
	// (`pod_exit_policy`), so an explicit field is what keeps the two
	// paths from drifting on the same project.
	pod.add("PodmanArgs", "--exit-policy=continue".to_string());

	let mut contents = owner_marker(project);
	contents.push_str(&pod.render());
	Some(QuadletUnit {
		filename: format!("{}.pod", safe_unit_stem(project)),
		contents,
	})
}

#[cfg(test)]
#[path = "pod_tests.rs"]
mod tests;
