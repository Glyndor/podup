use podup::compose::types::{ComposeFile, Service};

use super::{finding, Finding};

/// The other container a mode joins: `container:<id>`, or `service:<name>`, which
/// the engine turns into the container of that service. Same as the runtime detector.
fn shared_target(mode: &str) -> Option<(&'static str, &str)> {
	mode.strip_prefix("container:")
		.map(|id| ("container", id))
		.or_else(|| mode.strip_prefix("service:").map(|name| ("service", name)))
}

/// Host-binding namespacing modes: `pid`, `ipc`, `uts`, `cgroup`, `userns_mode`
/// set to `host` or `container:<id>`, or `network_mode` carrying the same. One
/// finding per active mode. The `container:<id>` form is the share-another-
/// container mode the runtime detector in
/// `internal/backends/engine/workload/container/warnings/host_mode.rs` already warns on; the audit
/// detector must agree, otherwise `podup audit --strict` would pass a file
/// the engine later refuses to run silently (#1746).
pub fn check_host_namespace(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	let mut out = Vec::new();
	if let Some(mode) = service.network_mode.as_deref() {
		if mode == "host" {
			out.push(finding(
				name,
				"host_namespace",
				"network_mode: host shares the host's network namespace",
			));
		} else if let Some((scheme, target)) = shared_target(mode) {
			out.push(finding(
				name,
				"host_namespace",
				&format!(
					"network_mode: {scheme}:{target} shares another container's network namespace"
				),
			));
		}
	}
	for field in ["pid", "ipc", "uts", "cgroup", "userns_mode"] {
		let value = match field {
			"pid" => &service.pid,
			"ipc" => &service.ipc,
			"uts" => &service.uts,
			"cgroup" => &service.cgroup,
			"userns_mode" => &service.userns_mode,
			_ => unreachable!("checked field list"),
		};
		if let Some(mode) = value.as_deref() {
			if mode == "host" {
				out.push(finding(
					name,
					"host_namespace",
					&format!("{field}: host shares the host's {field} namespace"),
				));
			} else if let Some((scheme, target)) = shared_target(mode) {
				out.push(finding(
					name,
					"host_namespace",
					&format!(
						"{field}: {scheme}:{target} shares another container's {field} namespace"
					),
				));
			}
		}
	}
	out
}

/// I flag an absent user namespace choice: rootless Podman's default maps
/// container root to the invoking user, without allocating a private range.
/// I point to docs/docker-migration.md for an explicit `auto` choice.
pub fn check_no_userns(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	if service.userns_mode.is_none() {
		vec![finding(
			name,
			"no_userns",
			"userns_mode is not set: rootless Podman's default maps container root to your host user; set `auto` explicitly for a private subordinate UID range; see docs/docker-migration.md",
		)]
	} else {
		Vec::new()
	}
}
