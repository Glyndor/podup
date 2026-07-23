//! Unknown-key warnings for nested compose *option blocks*.
//!
//! The typed [`ComposeFile`](crate::compose::types::ComposeFile) silently drops
//! any key it does not model inside seven nested option blocks — a `bind`,
//! `volume`, or `tmpfs` mount block, a long-form service `networks.<net>` map,
//! and the `deploy.resources.{limits,reservations}` specs (plus their
//! `driver_config` / `devices[]` children). Unlike the service- and top-level
//! passes, these structs carry no `#[serde(flatten)]` unknown bucket (adding one
//! would change a public type and break the 1.x SemVer gate), so the dropped
//! keys are unreachable from the parsed model.
//!
//! This pass therefore works from the *raw, interpolated* YAML document instead.
//! For every such block it compares the present keys against an explicit
//! allowlist of the keys that block's type models: a key that is neither modeled
//! nor an `x-` extension is reported.
//!
//! The allowlist is deliberate, not derived from a round-trip. A round-trip
//! (serialize-the-parsed-struct) would drop any modeled key whose value is
//! `None`/empty (every field carries `skip_serializing_if`), so `propagation:`
//! null, `link_local_ips: []`, `driver_opts: {}`, or `devices: []` would be
//! mis-flagged as unknown — the forbidden "warn on a modeled key" case. A guard
//! test per type (see `tests`) serializes a fully-populated, exhaustive struct
//! literal and asserts its key set equals the allowlist, so adding a field to
//! any of the seven structs fails to compile until both the literal and the
//! allowlist are updated.

// --- Per-type allowlists of modeled serde keys -----------------------------
//
// Each entry is the YAML key serde reads/writes for the field (accounting for
// any `#[serde(rename)]`; none of the seven currently rename). Kept in sync with
// the structs by the exhaustive-literal guard tests below.

/// `BindOptions` (volume.rs).
const BIND_OPTIONS_KEYS: &[&str] = &["propagation", "create_host_path", "selinux"];
/// `VolumeOptions` (volume.rs).
const VOLUME_OPTIONS_KEYS: &[&str] = &[
	"nocopy",
	"labels",
	"driver_config",
	"subpath",
	"noexec",
	"nosuid",
	"nodev",
];
/// `DriverConfig` (volume.rs).
const DRIVER_CONFIG_KEYS: &[&str] = &["name", "options"];
/// `TmpfsOptions` (volume.rs).
const TMPFS_OPTIONS_KEYS: &[&str] = &["size", "mode"];
/// `ServiceNetworkConfig` (network.rs).
const SERVICE_NETWORK_CONFIG_KEYS: &[&str] = &[
	"aliases",
	"ipv4_address",
	"ipv6_address",
	"link_local_ips",
	"priority",
	"mac_address",
	"driver_opts",
	"gw_priority",
	"interface_name",
];
/// `ResourceSpec` (deploy.rs).
const RESOURCE_SPEC_KEYS: &[&str] = &["cpus", "memory", "pids", "devices"];
/// `DeviceReservation` (deploy.rs).
const DEVICE_RESERVATION_KEYS: &[&str] =
	&["capabilities", "count", "device_ids", "driver", "options"];

/// Collect unknown-key warnings for every nested option block in an already
/// interpolated, merge-resolved compose document.
///
/// Pure (no I/O, no logging) so it is unit-testable; the caller emits each
/// message via `tracing::warn!`. An unparseable document yields no warnings — it
/// is the parser proper's job to report that.
pub(crate) fn raw_nested_unknown_warnings(interpolated_yaml: &str) -> Vec<String> {
	let mut out = Vec::new();
	let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(interpolated_yaml) else {
		return out;
	};
	let Some(services) = doc.get("services").and_then(|v| v.as_mapping()) else {
		return out;
	};
	for (name, def) in services {
		let (Some(service), Some(svc)) = (name.as_str(), def.as_mapping()) else {
			continue;
		};
		walk_volumes(service, svc, &mut out);
		walk_networks(service, svc, &mut out);
		walk_deploy(service, svc, &mut out);
	}
	out
}

/// `services.<svc>.volumes[i].{bind,volume,tmpfs}` (long-form mounts only).
fn walk_volumes(service: &str, svc: &serde_yaml::Mapping, out: &mut Vec<String>) {
	let Some(mounts) = svc.get("volumes").and_then(|v| v.as_sequence()) else {
		return;
	};
	for (i, mount) in mounts.iter().enumerate() {
		// Short-form `"src:dst"` string mounts have no option block.
		let Some(m) = mount.as_mapping() else {
			continue;
		};
		if let Some(bind) = m.get("bind").and_then(|v| v.as_mapping()) {
			diff_unknown(
				bind,
				BIND_OPTIONS_KEYS,
				&format!("service '{service}' volumes[{i}].bind"),
				out,
			);
		}
		if let Some(volume) = m.get("volume").and_then(|v| v.as_mapping()) {
			diff_unknown(
				volume,
				VOLUME_OPTIONS_KEYS,
				&format!("service '{service}' volumes[{i}].volume"),
				out,
			);
			// `driver_config` is itself an option block; the parent allowlist only
			// records its presence, so recurse to reach its own unknown keys.
			if let Some(dc) = volume.get("driver_config").and_then(|v| v.as_mapping()) {
				diff_unknown(
					dc,
					DRIVER_CONFIG_KEYS,
					&format!("service '{service}' volumes[{i}].volume.driver_config"),
					out,
				);
			}
		}
		if let Some(tmpfs) = m.get("tmpfs").and_then(|v| v.as_mapping()) {
			diff_unknown(
				tmpfs,
				TMPFS_OPTIONS_KEYS,
				&format!("service '{service}' volumes[{i}].tmpfs"),
				out,
			);
		}
	}
}

/// `services.<svc>.networks.<net>` — only the long-form mapping carries options;
/// a bare list or a `null` attachment has nothing to diff.
fn walk_networks(service: &str, svc: &serde_yaml::Mapping, out: &mut Vec<String>) {
	let Some(nets) = svc.get("networks").and_then(|v| v.as_mapping()) else {
		return;
	};
	for (net, cfg) in nets {
		let (Some(net), Some(cfg)) = (net.as_str(), cfg.as_mapping()) else {
			continue;
		};
		diff_unknown(
			cfg,
			SERVICE_NETWORK_CONFIG_KEYS,
			&format!("service '{service}' networks.{net}"),
			out,
		);
	}
}

/// `services.<svc>.deploy.resources.{limits,reservations}` and their
/// `devices[]` children.
fn walk_deploy(service: &str, svc: &serde_yaml::Mapping, out: &mut Vec<String>) {
	let Some(resources) = svc
		.get("deploy")
		.and_then(|v| v.as_mapping())
		.and_then(|d| d.get("resources"))
		.and_then(|r| r.as_mapping())
	else {
		return;
	};
	for kind in ["limits", "reservations"] {
		let Some(spec) = resources.get(kind).and_then(|v| v.as_mapping()) else {
			continue;
		};
		diff_unknown(
			spec,
			RESOURCE_SPEC_KEYS,
			&format!("service '{service}' deploy.resources.{kind}"),
			out,
		);
		let Some(devices) = spec.get("devices").and_then(|v| v.as_sequence()) else {
			continue;
		};
		for (j, dev) in devices.iter().enumerate() {
			if let Some(d) = dev.as_mapping() {
				diff_unknown(
					d,
					DEVICE_RESERVATION_KEYS,
					&format!("service '{service}' deploy.resources.{kind}.devices[{j}]"),
					out,
				);
			}
		}
	}
}

/// Report every key in `m` that is neither in `known` (the type's modeled serde
/// keys) nor an `x-` extension. No deserialization is involved: comparing
/// against the explicit allowlist means a modeled key is never flagged, even
/// when its value is null/empty and would have been dropped by a round-trip.
fn diff_unknown(m: &serde_yaml::Mapping, known: &[&str], context: &str, out: &mut Vec<String>) {
	for key in m.keys() {
		let Some(key) = key.as_str() else {
			continue;
		};
		if key.starts_with("x-") || known.contains(&key) {
			continue;
		}
		out.push(format!(
			"{context}: unknown key '{key}' is ignored \
			 (check for a typo or an unsupported compose feature)"
		));
	}
}

#[cfg(test)]
mod tests {
	use std::collections::{BTreeSet, HashMap};

	use super::*;
	use crate::compose::types::{
		BindOptions, CountOrAll, DeviceReservation, DriverConfig, Labels, ResourceSpec,
		ServiceNetworkConfig, TmpfsOptions, VolumeOptions,
	};

	/// Interpolate + merge-resolve `yaml` exactly as the parser does, then diff
	/// the nested blocks — mirrors the production caller, which feeds the pure
	/// entry the interpolated document text.
	fn warnings_for(yaml: &str) -> Vec<String> {
		let value = crate::compose::merge::interpolated_value(yaml, None).unwrap();
		let text = serde_yaml::to_string(&value).unwrap();
		raw_nested_unknown_warnings(&text)
	}

	/// Keys serde actually writes for a value, as a set.
	fn serialized_keys<T: serde::Serialize>(value: &T) -> BTreeSet<String> {
		serde_yaml::to_value(value)
			.unwrap()
			.as_mapping()
			.unwrap()
			.keys()
			.map(|k| k.as_str().unwrap().to_string())
			.collect()
	}

	fn allowlist(keys: &[&str]) -> BTreeSet<String> {
		keys.iter().map(|s| s.to_string()).collect()
	}

	fn one_entry_map() -> HashMap<String, String> {
		HashMap::from([("k".to_string(), "v".to_string())])
	}

	// --- Drift guards: an exhaustive struct literal (no `..Default::default()`)
	// forces a compile error if a field is added, until the allowlist is updated.

	#[test]
	fn bind_options_allowlist_matches_serde() {
		let v = BindOptions {
			propagation: Some("rprivate".to_string()),
			create_host_path: Some(true),
			selinux: Some("z".to_string()),
		};
		assert_eq!(serialized_keys(&v), allowlist(BIND_OPTIONS_KEYS));
	}

	#[test]
	fn volume_options_allowlist_matches_serde() {
		let v = VolumeOptions {
			nocopy: Some(true),
			labels: Labels::List(vec!["a=b".to_string()]),
			driver_config: Some(DriverConfig {
				name: Some("local".to_string()),
				options: one_entry_map(),
			}),
			subpath: Some("sub".to_string()),
			noexec: Some(true),
			nosuid: Some(true),
			nodev: Some(true),
		};
		assert_eq!(serialized_keys(&v), allowlist(VOLUME_OPTIONS_KEYS));
	}

	#[test]
	fn driver_config_allowlist_matches_serde() {
		let v = DriverConfig {
			name: Some("local".to_string()),
			options: one_entry_map(),
		};
		assert_eq!(serialized_keys(&v), allowlist(DRIVER_CONFIG_KEYS));
	}

	#[test]
	fn tmpfs_options_allowlist_matches_serde() {
		let v = TmpfsOptions {
			size: Some(1024),
			mode: Some(0o755),
		};
		assert_eq!(serialized_keys(&v), allowlist(TMPFS_OPTIONS_KEYS));
	}

	#[test]
	fn service_network_config_allowlist_matches_serde() {
		let v = ServiceNetworkConfig {
			aliases: Some(vec!["a".to_string()]),
			ipv4_address: Some("10.0.0.2".to_string()),
			ipv6_address: Some("::1".to_string()),
			link_local_ips: vec!["169.254.0.1".to_string()],
			priority: Some(1),
			mac_address: Some("02:42:ac:11:00:02".to_string()),
			driver_opts: one_entry_map(),
			gw_priority: Some(2),
			interface_name: Some("eth0".to_string()),
		};
		assert_eq!(serialized_keys(&v), allowlist(SERVICE_NETWORK_CONFIG_KEYS));
	}

	#[test]
	fn resource_spec_allowlist_matches_serde() {
		let v = ResourceSpec {
			cpus: Some("0.5".to_string()),
			memory: Some("512M".to_string()),
			pids: Some(100),
			devices: vec![DeviceReservation {
				capabilities: vec!["gpu".to_string()],
				count: Some(CountOrAll::N(1)),
				device_ids: vec!["0".to_string()],
				driver: Some("nvidia".to_string()),
				options: one_entry_map(),
			}],
		};
		assert_eq!(serialized_keys(&v), allowlist(RESOURCE_SPEC_KEYS));
	}

	#[test]
	fn device_reservation_allowlist_matches_serde() {
		let v = DeviceReservation {
			capabilities: vec!["gpu".to_string()],
			count: Some(CountOrAll::N(1)),
			device_ids: vec!["0".to_string()],
			driver: Some("nvidia".to_string()),
			options: one_entry_map(),
		};
		assert_eq!(serialized_keys(&v), allowlist(DEVICE_RESERVATION_KEYS));
	}

	// --- Positive: an unknown key in each block warns with the right context ---

	#[test]
	fn warns_on_unknown_bind_key() {
		// `create_hostpath` is a typo for `create_host_path` and is dropped silently
		// by `BindOptions`; it must be surfaced with the indexed context.
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - type: bind\n        source: /host\n        target: /in\n        bind:\n          create_hostpath: true\n",
		);
		assert!(
			msgs.iter().any(|m| m
				== "service 'web' volumes[0].bind: unknown key 'create_hostpath' is ignored (check for a typo or an unsupported compose feature)"),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_unknown_volume_key() {
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - type: volume\n        source: data\n        target: /data\n        volume:\n          nocpy: true\n",
		);
		assert!(
			msgs.iter()
				.any(|m| m.contains("volumes[0].volume") && m.contains("nocpy")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_unknown_tmpfs_key() {
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - type: tmpfs\n        target: /t\n        tmpfs:\n          siz: 1024\n",
		);
		assert!(
			msgs.iter()
				.any(|m| m.contains("volumes[0].tmpfs") && m.contains("siz")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_unknown_driver_config_key_via_recursion() {
		// The unknown key lives one level below `volume`, which the parent
		// allowlist cannot reach — only the recursion into `driver_config` finds it.
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - type: volume\n        source: data\n        target: /data\n        volume:\n          driver_config:\n            name: local\n            optoins: {}\n",
		);
		assert!(
			msgs.iter()
				.any(|m| m.contains("volumes[0].volume.driver_config") && m.contains("optoins")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_unknown_deploy_limits_key() {
		let msgs = warnings_for(
			"services:\n  db:\n    image: pg\n    deploy:\n      resources:\n        limits:\n          cpus: '0.5'\n          memroy: 512M\n",
		);
		assert!(
			msgs.iter().any(|m| m
				== "service 'db' deploy.resources.limits: unknown key 'memroy' is ignored (check for a typo or an unsupported compose feature)"),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_unknown_reservations_device_key_via_recursion() {
		let msgs = warnings_for(
			"services:\n  db:\n    image: pg\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [gpu]\n              cont: 1\n",
		);
		assert!(
			msgs.iter()
				.any(|m| m.contains("deploy.resources.reservations.devices[0]")
					&& m.contains("cont")),
			"got: {msgs:?}"
		);
	}

	#[test]
	fn warns_on_unknown_service_network_key() {
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    networks:\n      frontend:\n         alises: [a]\nnetworks:\n  frontend:\n",
		);
		assert!(
			msgs.iter().any(|m| m
				== "service 'web' networks.frontend: unknown key 'alises' is ignored (check for a typo or an unsupported compose feature)"),
			"got: {msgs:?}"
		);
	}

	// --- x- extension keys are never flagged -----------------------------------

	#[test]
	fn x_extension_key_in_a_block_is_not_flagged() {
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - type: bind\n        source: /host\n        target: /in\n        bind:\n          x-foo: bar\n",
		);
		assert!(
			!msgs.iter().any(|m| m.contains("x-foo")),
			"x- extension keys must never be flagged; got: {msgs:?}"
		);
	}

	// --- Negative: modeled keys (incl. empty/null values) never warn -----------

	#[test]
	fn fully_modeled_blocks_produce_no_warning() {
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - type: bind\n        source: /host\n        target: /in\n        bind:\n          propagation: rprivate\n          create_host_path: true\n          selinux: z\n    networks:\n      frontend:\n        aliases: [web]\n        ipv4_address: 10.0.0.2\n    deploy:\n      resources:\n        limits:\n          cpus: '0.5'\n          memory: 512M\n          pids: 100\nnetworks:\n  frontend:\n",
		);
		assert!(msgs.is_empty(), "unexpected warnings: {msgs:?}");
	}

	#[test]
	fn modeled_key_with_null_value_does_not_warn() {
		// `propagation:` is a modeled key whose value is null; a round-trip would
		// drop it (skip_serializing_if) and mis-flag it, but the allowlist must not.
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - type: bind\n        source: /host\n        target: /in\n        bind:\n          propagation:\n",
		);
		assert!(
			!msgs.iter().any(|m| m.contains("propagation")),
			"a modeled-but-null key must not warn; got: {msgs:?}"
		);
	}

	#[test]
	fn modeled_keys_with_empty_collections_do_not_warn() {
		// Empty modeled collections — `link_local_ips: []`, `driver_opts: {}` on a
		// service network and `devices: []` on a reservation — would all be dropped
		// by a round-trip; none may warn.
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    networks:\n      frontend:\n        aliases: []\n        link_local_ips: []\n        driver_opts: {}\n    deploy:\n      resources:\n        reservations:\n          devices: []\nnetworks:\n  frontend:\n",
		);
		assert!(msgs.is_empty(), "unexpected warnings: {msgs:?}");
	}

	#[test]
	fn modeled_empty_options_in_driver_config_does_not_warn() {
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - type: volume\n        source: data\n        target: /data\n        volume:\n          driver_config:\n            name: local\n            options: {}\n",
		);
		assert!(
			!msgs.iter().any(|m| m.contains("options")),
			"a modeled-but-empty map must not warn; got: {msgs:?}"
		);
	}

	#[test]
	fn clean_file_produces_no_warning() {
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - ./data:/app/data\n",
		);
		assert!(msgs.is_empty(), "unexpected warnings: {msgs:?}");
	}

	#[test]
	fn null_network_attachment_is_not_a_block() {
		// `networks: { frontend: }` is a null attachment, not an options map — there
		// is nothing to diff and it must not warn.
		let msgs = warnings_for(
			"services:\n  web:\n    image: nginx\n    networks:\n      frontend:\nnetworks:\n  frontend:\n",
		);
		assert!(msgs.is_empty(), "unexpected warnings: {msgs:?}");
	}

	#[test]
	fn unparseable_document_yields_no_warnings() {
		assert!(raw_nested_unknown_warnings(": : :").is_empty());
	}
}
