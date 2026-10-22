use super::*;

/// Pod create body carries the project name, the two `podup.*` labels, the
/// shared `net` namespace, the union of every service's port set, the
/// declared networks, and one `hostadd` per service.
#[tokio::test]
#[cfg(unix)]
async fn pod_create_body_carries_name_labels_ports_networks_and_hosts() {
	let fake = pod_up_fake();
	let engine = engine_for(&fake, "proj");
	let yaml = r#"
x-podman-pod: true
services:
  web:
    image: nginx
    ports: ["8080:80"]
  db:
    image: postgres
    ports: ["5432:5432"]
networks:
  backend:
"#;
	let file = parse_str(yaml).expect("compose must parse");
	engine.up(&file).await.expect("up must succeed");

	let bodies = fake.bodies.lock().unwrap().clone();
	let pod_body = decode_body(&bodies, "pods");

	assert_eq!(pod_body["name"], "proj");
	assert_eq!(pod_body["labels"]["podup.project"], "proj");
	assert!(
		pod_body["labels"]["podup.pod-config-hash"]
			.as_str()
			.map(|s| !s.is_empty())
			.unwrap_or(false),
		"pod must carry a pod-config-hash label"
	);
	assert_eq!(pod_body["shared_namespaces"], serde_json::json!(["net"]));
	// A pod that attaches to networks must ask for bridge networking, or libpod refuses it.
	assert_eq!(pod_body["netns"], serde_json::json!({"nsmode": "bridge"}));
	assert!(
		pod_body["networks"].get("proj_backend").is_some(),
		"the pod must attach to the declared network: {pod_body}"
	);
	// Union of every service's port set: 80, 5432 container ports; 8080, 5432 host ports.
	let ports = pod_body["portmappings"]
		.as_array()
		.expect("portmappings must be an array");
	let container_ports: Vec<u16> = ports
		.iter()
		.map(|p| p["container_port"].as_u64().unwrap() as u16)
		.collect();
	assert!(container_ports.contains(&80));
	assert!(container_ports.contains(&5432));
	// Host entries: one per service.
	let hosts: Vec<String> = pod_body["hostadd"]
		.as_array()
		.expect("hostadd must be an array")
		.iter()
		.map(|v| v.as_str().unwrap().to_string())
		.collect();
	assert!(hosts.contains(&"web:127.0.0.1".to_string()));
	assert!(hosts.contains(&"db:127.0.0.1".to_string()));
	// Exit policy: the spec sets it explicitly so the API path does not
	// inherit a `containers.conf` `pod_exit_policy`. The Quadlet path
	// emits the same value as a `PodmanArgs=` flag, so both paths agree
	// on the same project.
	assert_eq!(
		pod_body["exit_policy"], "continue",
		"the pod create body must carry exit_policy=continue: {pod_body}"
	);
}

/// Containers created in pod mode carry `pod` set to the project name and
/// have no per-container portmappings, networks, or netns.
#[tokio::test]
#[cfg(unix)]
async fn pod_containers_carry_pod_and_no_ports_or_networks() {
	let fake = pod_up_fake();
	let engine = engine_for(&fake, "proj");
	let yaml = r#"
x-podman-pod: true
services:
  web:
    image: nginx
    ports: ["8080:80"]
  db:
    image: postgres
"#;
	let file = parse_str(yaml).expect("compose must parse");
	engine.up(&file).await.expect("up must succeed");

	let bodies = fake.bodies.lock().unwrap().clone();
	// Every container create body should carry `pod` and an empty
	// portmappings/networks. Pick any one (the first non-empty body).
	let pod_body = decode_body(&bodies, "pod");
	assert_eq!(pod_body["pod"], "proj");
	// `portmappings`, `networks` and `netns` carry
	// `skip_serializing_if = ...is_empty/is_none`, so an empty list is
	// omitted from the JSON body rather than serialised as `[]`/`{}`.
	assert!(
		pod_body.get("portmappings").is_none() || pod_body["portmappings"].is_null(),
		"container inside a pod cannot publish ports, got: {pod_body}"
	);
	assert!(
		pod_body.get("networks").is_none() || pod_body["networks"].is_null(),
		"container inside a pod cannot attach networks, got: {pod_body}"
	);
	assert!(
		pod_body.get("netns").is_none() || pod_body["netns"].is_null(),
		"container inside a pod must not carry its own netns, got: {pod_body}"
	);
}
