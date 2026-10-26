use super::*;

/// Adding a port changes the pod's hash; changing the service's command does
/// not.
#[tokio::test]
#[cfg(unix)]
async fn pod_hash_changes_with_a_port_and_not_with_a_command() {
	let yaml_base = r#"
x-podman-pod: true
services:
  web:
    image: nginx
"#;
	let file_base = parse_str(yaml_base).unwrap();
	let base_ports = vec![crate::ports::parse_ports(&file_base.services["web"].ports).unwrap()];

	let yaml_with_port = r#"
x-podman-pod: true
services:
  web:
    image: nginx
    ports: ["8080:80"]
"#;
	let file_with_port = parse_str(yaml_with_port).unwrap();
	let with_port_ports =
		vec![crate::ports::parse_ports(&file_with_port.services["web"].ports).unwrap()];

	let yaml_with_command = r#"
x-podman-pod: true
services:
  web:
    image: nginx
    command: ["nginx", "-g", "daemon off;"]
"#;
	let file_with_command = parse_str(yaml_with_command).unwrap();
	let with_command_ports =
		vec![crate::ports::parse_ports(&file_with_command.services["web"].ports).unwrap()];

	let base = crate::engine::pod::pod_config_hash(&base_ports, &file_base);
	let with_port = crate::engine::pod::pod_config_hash(&with_port_ports, &file_with_port);
	let with_command = crate::engine::pod::pod_config_hash(&with_command_ports, &file_with_command);

	assert_ne!(base, with_port, "adding a port must change the pod hash");
	assert_eq!(
		base, with_command,
		"changing the service command must NOT change the pod hash"
	);
}

/// When the recorded hash differs from the new one, the pod is recreated:
/// `DELETE /pods/{name}?force=true` then `POST /pods/create`.
#[tokio::test]
#[cfg(unix)]
async fn pod_is_recreated_when_the_hash_differs() {
	let fake = pod_up_fake_with_existing(Some("wrong-hash"));
	let engine = engine_for(&fake, "proj");
	let yaml = r#"
x-podman-pod: true
services:
  web:
    image: nginx
"#;
	let file = parse_str(yaml).unwrap();
	engine.up(&file).await.expect("up must succeed");

	let requests = fake.requests.lock().unwrap().clone();
	let deleted_pod = requests
		.iter()
		.any(|r| r.starts_with("DELETE") && r.contains("/pods/proj") && r.contains("force=true"));
	let created_pod = requests
		.iter()
		.any(|r| r.starts_with("POST") && r.contains("/pods/create"));
	assert!(
		deleted_pod && created_pod,
		"a pod with a mismatching hash must be removed and recreated; requests: {requests:?}"
	);
}

/// When the recorded hash matches, the engine does NOT re-create the pod.
#[tokio::test]
#[cfg(unix)]
async fn pod_is_reused_when_the_hash_matches() {
	// Compute the expected hash for this compose file and feed it back as
	// the recorded hash so the engine reuses the pod.
	let yaml = r#"
x-podman-pod: true
services:
  web:
    image: nginx
    ports: ["8080:80"]
"#;
	let file = parse_str(yaml).unwrap();
	let expected = crate::engine::pod::pod_config_hash(
		&[crate::ports::parse_ports(&file.services["web"].ports).unwrap()],
		&file,
	);
	let fake = pod_up_fake_with_existing(Some(&expected));
	let engine = engine_for(&fake, "proj");
	engine.up(&file).await.expect("up must succeed");

	let requests = fake.requests.lock().unwrap().clone();
	let deleted_pod = requests
		.iter()
		.any(|r| r.starts_with("DELETE") && r.contains("/pods/proj") && r.contains("force=true"));
	let created_pod = requests
		.iter()
		.filter(|r| r.starts_with("POST") && r.contains("/pods/create"))
		.count();
	let inspected_pod = requests
		.iter()
		.filter(|r| r.starts_with("GET") && r.contains("/pods/") && r.contains("/json"))
		.count();
	assert!(
		!deleted_pod,
		"a matching-hash pod must not be removed; requests: {requests:?}"
	);
	assert_eq!(
		created_pod, 0,
		"a matching-hash pod must NOT be created again, but a recreate would also POST; requests: {requests:?}"
	);
	assert!(
		inspected_pod >= 1,
		"the engine must inspect the pod to read its hash before deciding; requests: {requests:?}"
	);
}
