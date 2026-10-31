use super::*;

// --- container label resolution ---

#[test]
fn resolve_container_labels_keeps_service_labels() {
	use crate::compose::types::primitives::Labels;
	use indexmap::IndexMap;
	let mut svc = default_service();
	let mut map = IndexMap::new();
	map.insert("com.example.team".to_string(), "blue".to_string());
	svc.labels = Labels::Map(map);

	let labels = resolve_container_labels(&svc, HashMap::new());
	assert_eq!(
		labels.get("com.example.team").map(String::as_str),
		Some("blue")
	);
}

#[test]
fn resolve_container_labels_does_not_apply_deploy_labels() {
	use crate::compose::types::primitives::Labels;
	use crate::compose::types::DeployConfig;
	use indexmap::IndexMap;
	let mut svc = default_service();
	let mut svc_map = IndexMap::new();
	svc_map.insert("com.example.service".to_string(), "on".to_string());
	svc.labels = Labels::Map(svc_map);
	let mut deploy_map = IndexMap::new();
	deploy_map.insert("com.example.deploy".to_string(), "swarm".to_string());
	svc.deploy = Some(DeployConfig {
		labels: Labels::Map(deploy_map),
		..Default::default()
	});

	let labels = resolve_container_labels(&svc, HashMap::new());
	// Per the Compose Specification, deploy.labels are NOT applied to the container.
	assert!(!labels.contains_key("com.example.deploy"));
	// Service labels still apply.
	assert_eq!(
		labels.get("com.example.service").map(String::as_str),
		Some("on")
	);
}

#[test]
fn resolve_container_labels_service_overrides_label_file() {
	use crate::compose::types::primitives::Labels;
	use indexmap::IndexMap;
	let mut svc = default_service();
	let mut map = IndexMap::new();
	map.insert("shared".to_string(), "from-service".to_string());
	svc.labels = Labels::Map(map);
	let mut file_labels = HashMap::new();
	file_labels.insert("shared".to_string(), "from-file".to_string());
	file_labels.insert("only-file".to_string(), "yes".to_string());

	let labels = resolve_container_labels(&svc, file_labels);
	assert_eq!(
		labels.get("shared").map(String::as_str),
		Some("from-service")
	);
	assert_eq!(labels.get("only-file").map(String::as_str), Some("yes"));
}
