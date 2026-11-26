//! Build the `.pod` Quadlet unit for a project.
//!
//! Emitted alongside the `.container` units when the compose file declares
//! `x-podman-pod: true`. The unit carries every port the pod publishes
//! (the union of every service's `ports:`), every declared network
//! (`Network=`), the pod-wide fields derived from the services and the
//! project ownership label; each `.container` unit references this pod by
//! `Pod=<stem>.pod` and drops the fields the pod carries. The `.container`
//! side of that contract is in `super::container::container_unit_with_mode`.
//!
//! Every promoted pod field is written as its native `[Pod]` key. Only the
//! pod's namespace sharing, which Quadlet has no key for, goes through
//! `PodmanArgs=`.

use crate::compose::types::ComposeFile;
use crate::ports;

mod fields;

pub(super) use fields::PodField;
pub(crate) use fields::PodFields;

use super::super::ownership;
use super::{owner_marker, render_publish_port, safe_unit_stem, unit_stem, QuadletUnit, Section};

/// Build the `.pod` unit for one project. The contents are a single
/// `[Pod]` section with `PodName=`, one `Network=` per declared network,
/// one `PublishPort=` per port (the union of every service's `ports:`),
/// every pod-level field derived from the services and one `--add-host` flag
/// per service. Each `.container` unit references this pod by `Pod=<stem>.pod`;
/// see [`super::container::container_unit_with_mode`].
///
/// An `installed` unit, the one quadlet-mode autostart writes, also carries the
/// lines that keep it off a pod of another project (`super::super::ownership`).
///
/// Returns `None` when the project has no pod-mode extension, so callers
/// can splice the unit into the output list without a conditional.
pub(crate) fn pod_unit(
	project: &str,
	file: &ComposeFile,
	pod_fields: &PodFields,
	installed: bool,
) -> Option<QuadletUnit> {
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

	// Podman's default shares the network, UTS and IPC namespaces. The live
	// engine shares only the network so every member keeps its own hostname
	// and IPC namespace. Measured on Podman 6.2.0 (2026-11-22): Quadlet has
	// no native key for this setting (the generator rejects `Share=`), so
	// pass Podman's `--share` through `PodmanArgs=`.
	pod.add("PodmanArgs", "--share=net".to_string());

	// Every non-external network the file declares, plus the external ones by
	// their own name. Mirrors the live engine's `pod_networks` builder.
	// When every service agreed on a pasta/slirp4netns mode the pod runs on
	// that mode instead, so no project networks are attached (libpod refuses
	// the combination). The mode is written as-is so the Podman CLI sees
	// `pasta:...` and splits it the way it does for `podman pod create
	// --network pasta:...`; Quadlet also passes the value through unchanged.
	if let Some(mode) = crate::engine::pod_network_mode(file) {
		pod.add("Network", mode.to_string());
	} else {
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

	pod_fields.emit(&mut pod);

	// One `<service>:127.0.0.1` per service, so a compose `db:5432` reference
	// resolves to the shared namespace the way it resolves on a project
	// network. The live engine builds the same map.
	let mut hosts: Vec<String> = file
		.services
		.keys()
		.map(|s| format!("{s}:127.0.0.1"))
		.collect();
	hosts.sort();
	for entry in &hosts {
		pod.add_native("AddHost", "--add-host=", entry);
	}

	// Carry the project ownership label the live engine stamps onto pods, so
	// `down`/`down --remove-orphans` can find the unit by label.
	pod.add("Label", format!("podup.project={project}"));

	// Pin the pod's exit policy to `continue` so the unit Quadlet
	// generates agrees with what the live engine stamps on the pod it
	// creates. Quadlet's default is `--exit-policy stop`, so the key is
	// always written: the exported pod keeps running when its last service
	// container exits. The live engine sets the same value explicitly: the API
	// path defaults to `continue`, but that default comes from `containers.conf`
	// (`pod_exit_policy`), so an explicit field is what keeps the two paths
	// from drifting on the same project.
	pod.add("ExitPolicy", "continue".to_string());

	let mut service = Section::new("Service");
	if installed {
		ownership::emit_pod(project, project, &mut pod, &mut service);
	}
	let mut contents = owner_marker(project);
	contents.push_str(&pod.render());
	if !service.is_empty() {
		contents.push('\n');
		contents.push_str(&service.render());
	}
	Some(QuadletUnit {
		filename: format!("{}.pod", safe_unit_stem(project)),
		contents,
	})
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
mod fields_conflict_tests;
#[cfg(test)]
mod fields_container_tests;
#[cfg(test)]
mod fields_dns_tests;
#[cfg(test)]
mod fields_golden_tests;
#[cfg(test)]
mod fields_mount_tests;
#[cfg(test)]
mod fields_network_tests;
#[cfg(test)]
mod fields_normalization_tests;
#[cfg(test)]
mod fields_test_support;
#[cfg(test)]
mod fields_write_tests;
#[cfg(test)]
mod share_tests;
