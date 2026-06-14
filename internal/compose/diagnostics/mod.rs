//! Parse-time diagnostics.
//!
//! podup accepts the full compose-spec surface and, per the spec's
//! forward-compatibility rule, never treats an unknown key as a hard error.
//! The cost of that leniency is silent drops: a typo or an unmapped compose
//! feature would just vanish. This pass closes that gap by reporting every key
//! or field podup parses but cannot translate, so nothing is ignored without
//! the operator hearing about it — the same guarantee that lets podup absorb
//! future Docker/Podman compose additions gracefully.

use super::types::ComposeFile;

mod ignored_fields;
use ignored_fields::{
	ignored_build_fields, ignored_network_fields, ignored_port_fields,
	ignored_secret_config_drivers, ignored_service_fields, ignored_service_network_fields,
	ignored_volume_mount_fields,
};

/// Collect a warning for every parsed-but-unsupported key or field in `file`.
/// Pure (no logging) so it can be unit-tested; the caller emits the messages.
pub(super) fn collect(file: &ComposeFile) -> Vec<String> {
	let mut out = Vec::new();
	unknown_top_level_keys(file, &mut out);
	unknown_service_keys(file, &mut out);
	nested_unknown_keys(file, &mut out);
	ignored_service_fields(file, &mut out);
	ignored_port_fields(file, &mut out);
	ignored_volume_mount_fields(file, &mut out);
	ignored_build_fields(file, &mut out);
	ignored_network_fields(file, &mut out);
	ignored_service_network_fields(file, &mut out);
	ignored_secret_config_drivers(file, &mut out);
	out
}

/// Push a warning for each non-`x-` key captured at a nested level.
fn push_unknown(
	context: &str,
	unknown: &indexmap::IndexMap<String, serde_yaml::Value>,
	out: &mut Vec<String>,
) {
	for key in unknown.keys() {
		if key.starts_with("x-") {
			continue;
		}
		out.push(format!(
			"{context}: unknown key '{key}' is ignored \
			 (check for a typo or an unsupported compose feature)"
		));
	}
}

/// Unknown keys captured inside service sub-objects and top-level network /
/// volume definitions. Together with the service- and top-level passes this
/// means a typo or an unmapped future field at ANY modeled level is surfaced
/// rather than silently dropped.
fn nested_unknown_keys(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		if let Some(hc) = &def.healthcheck {
			push_unknown(
				&format!("service '{service}' healthcheck"),
				&hc.unknown,
				out,
			);
		}
		if let Some(deploy) = &def.deploy {
			push_unknown(&format!("service '{service}' deploy"), &deploy.unknown, out);
		}
		if let Some(develop) = &def.develop {
			for (i, rule) in develop.watch.iter().enumerate() {
				push_unknown(
					&format!("service '{service}' develop.watch[{i}]"),
					&rule.unknown,
					out,
				);
			}
		}
	}
	for (name, cfg) in &file.networks {
		if let Some(c) = cfg {
			push_unknown(&format!("network '{name}'"), &c.unknown, out);
			if let Some(ipam) = &c.ipam {
				push_unknown(&format!("network '{name}' ipam"), &ipam.unknown, out);
			}
		}
	}
	for (name, cfg) in &file.volumes {
		if let Some(c) = cfg {
			push_unknown(&format!("volume '{name}'"), &c.unknown, out);
		}
	}
}

/// Top-level keys that matched no known field (captured in `extensions`).
/// `x-*` extension keys are allowed by the spec and skipped.
fn unknown_top_level_keys(file: &ComposeFile, out: &mut Vec<String>) {
	for key in file.extensions.keys() {
		if key.starts_with("x-") {
			continue;
		}
		out.push(format!(
			"unknown top-level key '{key}' is ignored \
			 (check for a typo or an unsupported compose feature)"
		));
	}
}

/// Service keys that matched no known field. A likely typo — e.g.
/// `enviroment:` — is easy to miss when it just vanishes, so surface it.
fn unknown_service_keys(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		for key in def.unknown.keys() {
			if key.starts_with("x-") {
				continue;
			}
			out.push(format!(
				"service '{service}': unknown key '{key}' is ignored \
				 (check for a typo or an unsupported compose feature)"
			));
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::parse_str;

	fn diagnostics_for(yaml: &str) -> Vec<String> {
		let file = parse_str(yaml).unwrap();
		super::collect(&file)
	}

	#[test]
	fn warns_on_unknown_top_level_key_but_not_x_extension() {
		let msgs = diagnostics_for(
			"x-anchors: ok\nservies:\n  typo: 1\nservices:\n  web:\n    image: nginx\n",
		);
		assert!(
			msgs.iter()
				.any(|m| m.contains("unknown top-level key 'servies'")),
			"got: {msgs:?}"
		);
		assert!(!msgs.iter().any(|m| m.contains("x-anchors")));
	}

	#[test]
	fn warns_on_unknown_service_key_but_not_x_extension() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    enviroment:\n      A: 1\n    x-meta: ok\n",
		);
		assert!(msgs.iter().any(|m| m.contains("unknown key 'enviroment'")));
		assert!(!msgs.iter().any(|m| m.contains("x-meta")));
	}

	#[test]
	fn warns_on_windows_only_cpu_fields() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    cpu_count: 2\n    cpu_percent: 50\n",
		);
		assert!(msgs.iter().any(|m| m.contains("cpu_count")));
		assert!(msgs.iter().any(|m| m.contains("cpu_percent")));
	}

	#[test]
	fn warns_on_unmapped_build_fields() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    build:\n      context: .\n      privileged: true\n      isolation: chroot\n",
		);
		assert!(msgs.iter().any(|m| m.contains("build.privileged")));
		assert!(msgs.iter().any(|m| m.contains("build.isolation")));
	}

	#[test]
	fn warns_on_network_enable_ipv4() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\nnetworks:\n  net:\n    enable_ipv4: false\n",
		);
		assert!(msgs.iter().any(|m| m.contains("enable_ipv4")));
	}

	#[test]
	fn warns_on_unknown_key_in_healthcheck() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    healthcheck:\n      test: [\"CMD\", \"true\"]\n      retires: 3\n",
		);
		assert!(
			msgs.iter()
				.any(|m| m.contains("healthcheck") && m.contains("retires")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_unknown_key_in_network_and_ipam() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\nnetworks:\n  net:\n    drivr: bridge\n    ipam:\n      confg: []\n",
		);
		assert!(msgs
			.iter()
			.any(|m| m.contains("network 'net'") && m.contains("drivr")));
		assert!(msgs
			.iter()
			.any(|m| m.contains("ipam") && m.contains("confg")));
	}

	#[test]
	fn warns_on_unknown_key_in_volume() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\nvolumes:\n  data:\n    externl: true\n",
		);
		assert!(msgs
			.iter()
			.any(|m| m.contains("volume 'data'") && m.contains("externl")));
	}

	#[test]
	fn warns_on_service_network_gw_priority() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    networks:\n      net:\n        gw_priority: 10\nnetworks:\n  net:\n",
		);
		assert!(
			msgs.iter().any(|m| m.contains("gw_priority")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn file_secret_produces_no_diagnostics() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    file: ./tok.txt\n",
		);
		assert!(msgs.is_empty(), "unexpected diagnostics: {msgs:?}");
	}

	#[test]
	fn clean_file_produces_no_diagnostics() {
		let msgs = diagnostics_for("services:\n  web:\n    image: nginx\n    cpu_shares: 512\n");
		assert!(msgs.is_empty(), "unexpected diagnostics: {msgs:?}");
	}

	#[test]
	fn warns_on_attach() {
		let msgs = diagnostics_for("services:\n  web:\n    image: nginx\n    attach: false\n");
		assert!(
			msgs.iter().any(|m| m.contains("attach is not honored")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_long_port_mode() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    ports:\n      - target: 80\n        published: 8080\n        mode: host\n",
		);
		assert!(
			msgs.iter().any(|m| m.contains("port mode 'host'")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_per_mount_driver_config() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - type: volume\n        source: data\n        target: /data\n        volume:\n          driver_config:\n            name: local\nvolumes:\n  data:\n",
		);
		assert!(
			msgs.iter().any(|m| m.contains("per-mount driver_config")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_interface_name() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    networks:\n      net:\n        interface_name: eth9\nnetworks:\n  net:\n",
		);
		assert!(
			msgs.iter().any(|m| m.contains("interface_name")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_non_external_secret_driver() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    driver: vault\n",
		);
		assert!(
			msgs.iter().any(|m| m.contains("secret 'tok': driver")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn external_secret_driver_produces_no_diagnostic() {
		let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    external: true\n    driver: vault\n",
		);
		assert!(
			!msgs.iter().any(|m| m.contains("driver")),
			"unexpected: {msgs:?}"
		);
	}
}
