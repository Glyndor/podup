//! `shm_size` in a pod: Podman mounts the pod's `/dev/shm` in every member
//! and ignores the member's own size, so the value goes on the pod.

use crate::compose::parse_str;

fn spec_for(yaml: &str) -> crate::libpod::types::pod::PodSpecGenerator {
	let file = parse_str(yaml).unwrap();
	let ports: Vec<_> = file
		.services
		.values()
		.map(|s| crate::ports::parse_ports(&s.ports).unwrap())
		.collect();
	super::build_pod_spec_with_hash("demo", &file, &ports, "hash-abc")
}

fn hash_for(yaml: &str) -> String {
	let file = parse_str(yaml).unwrap();
	let ports: Vec<_> = file
		.services
		.values()
		.map(|s| crate::ports::parse_ports(&s.ports).unwrap())
		.collect();
	super::pod_config_hash(&ports, &file)
}

#[test]
fn an_agreed_shm_size_is_the_pods() {
	let spec = spec_for(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    shm_size: 128m\n  db:\n    image: postgres\n    shm_size: 128m\n",
	);
	assert_eq!(spec.shm_size, Some(128 * 1024 * 1024));
}

#[test]
fn a_service_without_shm_size_leaves_the_others_value() {
	let spec = spec_for(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    shm_size: 128m\n  db:\n    image: postgres\n",
	);
	assert_eq!(spec.shm_size, Some(128 * 1024 * 1024));
}

#[test]
fn disagreeing_services_get_the_largest_size() {
	let spec = spec_for(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    shm_size: 32m\n  db:\n    image: postgres\n    shm_size: 1g\n",
	);
	assert_eq!(spec.shm_size, Some(1024 * 1024 * 1024));
}

#[test]
fn no_shm_size_leaves_podmans_default() {
	let spec = spec_for("x-podman-pod: true\nservices:\n  web:\n    image: nginx\n");
	assert_eq!(spec.shm_size, None);
}

#[test]
fn the_pod_body_names_the_field_as_libpod_reads_it() {
	let spec =
		spec_for("x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    shm_size: 128m\n");
	let body = serde_json::to_value(&spec).unwrap();
	assert_eq!(body["shm_size"], 128 * 1024 * 1024);
}

#[test]
fn changing_shm_size_recreates_the_pod() {
	// The pod's /dev/shm is fixed at creation; a new size needs a new pod.
	let a =
		hash_for("x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    shm_size: 64m\n");
	let b =
		hash_for("x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    shm_size: 128m\n");
	assert_ne!(a, b);
}
