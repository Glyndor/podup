//! Serialisation tests for the libpod pod request/response types.

use std::collections::HashMap;

use super::{PodInspect, PodSpecGenerator};

#[test]
fn pod_spec_serialises_with_known_field_names() {
	// The wire keys are dictated by libpod; if any of these regresses to a
	// Rust name the daemon ignores the field silently.
	let mut labels = HashMap::new();
	labels.insert("podup.project".to_string(), "demo".to_string());
	labels.insert("podup.pod-config-hash".to_string(), "abc".to_string());
	let spec = PodSpecGenerator {
		netns: None,
		userns: None,
		exit_policy: None,
		name: "demo".to_string(),
		labels,
		shared_namespaces: vec!["net".to_string()],
		portmappings: vec![],
		networks: HashMap::new(),
		network_options: HashMap::new(),
		hostadd: vec![],
	};
	let json = serde_json::to_value(&spec).unwrap();
	assert_eq!(json["name"], "demo");
	assert_eq!(json["labels"]["podup.project"], "demo");
	assert_eq!(json["labels"]["podup.pod-config-hash"], "abc");
	assert_eq!(json["shared_namespaces"], serde_json::json!(["net"]));
}

/// A pod spec with `exit_policy` set serialises the field under its
/// libpod wire name (`exit_policy`), so the daemon reads it instead of
/// silently keeping the `containers.conf` default. Verified live against
/// `/v5.7.0/libpod/pods/create` before the field was added.
#[test]
fn pod_spec_serialises_exit_policy_when_set() {
	let spec = PodSpecGenerator {
		netns: None,
		userns: None,
		exit_policy: Some("continue".to_string()),
		name: "demo".to_string(),
		labels: HashMap::new(),
		shared_namespaces: vec![],
		portmappings: vec![],
		networks: HashMap::new(),
		network_options: HashMap::new(),
		hostadd: vec![],
	};
	let json = serde_json::to_value(&spec).unwrap();
	assert_eq!(json["exit_policy"], "continue");
}

/// A pod spec with `exit_policy: None` omits the key entirely, so a caller
/// that has no opinion lets libpod apply its default.
#[test]
fn pod_spec_omits_exit_policy_when_none() {
	let spec = PodSpecGenerator {
		netns: None,
		userns: None,
		exit_policy: None,
		name: "demo".to_string(),
		labels: HashMap::new(),
		shared_namespaces: vec![],
		portmappings: vec![],
		networks: HashMap::new(),
		network_options: HashMap::new(),
		hostadd: vec![],
	};
	let json = serde_json::to_value(&spec).unwrap();
	assert!(
		json.get("exit_policy").is_none(),
		"exit_policy must be skipped when None, got: {json}"
	);
}

/// A pod created on a user-mode network carries `netns` set to the bare
/// mode and `network_options` keyed by that mode, the split the Podman
/// CLI does for `podman pod create --network pasta:...`. The networks
/// map stays empty: libpod rejects networks on a non-bridge mode.
#[test]
fn pod_spec_serialises_network_mode_and_options() {
	let mut network_options = HashMap::new();
	network_options.insert(
		"pasta".to_string(),
		vec!["-m".to_string(), "1400".to_string()],
	);
	let spec = PodSpecGenerator {
		netns: Some(crate::libpod::types::container::Namespace::new("pasta")),
		userns: None,
		exit_policy: None,
		name: "demo".to_string(),
		labels: HashMap::new(),
		shared_namespaces: vec!["net".to_string()],
		portmappings: vec![],
		networks: HashMap::new(),
		network_options,
		hostadd: vec![],
	};
	let json = serde_json::to_value(&spec).unwrap();
	assert_eq!(json["netns"], serde_json::json!({"nsmode": "pasta"}));
	assert_eq!(json["network_options"]["pasta"][0], "-m");
	assert_eq!(json["network_options"]["pasta"][1], "1400");
	assert!(
		json.get("networks").is_none() || json["networks"].is_null(),
		"a pod on a non-bridge mode must not carry networks; got: {json}"
	);
}

#[test]
fn pod_inspect_reads_labels_and_name() {
	let json = r#"{
		"Name": "demo",
		"Labels": { "podup.pod-config-hash": "abc", "podup.project": "demo" },
		"NumContainers": 3
	}"#;
	let inspect: PodInspect = serde_json::from_str(json).unwrap();
	assert_eq!(
		inspect
			.labels
			.get("podup.pod-config-hash")
			.map(String::as_str),
		Some("abc"),
	);
	assert_eq!(
		inspect.labels.get("podup.project").map(String::as_str),
		Some("demo"),
	);
}
