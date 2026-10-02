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
		hostadd: vec![],
	};
	let json = serde_json::to_value(&spec).unwrap();
	assert!(
		json.get("exit_policy").is_none(),
		"exit_policy must be skipped when None, got: {json}"
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
