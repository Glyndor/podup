use super::*;

/// `down` removes the project's containers and THEN the pod.
#[tokio::test]
#[cfg(unix)]
async fn down_removes_the_pod_after_the_containers() {
	let fake = pod_up_fake();
	let engine = engine_for(&fake, "proj");
	let yaml = r#"
x-podman-pod: true
services:
  web:
    image: nginx
"#;
	let file = parse_str(yaml).unwrap();
	engine.up(&file).await.expect("up must succeed");
	engine.down(&file).await.expect("down must succeed");

	let requests = fake.requests.lock().unwrap().clone();
	let pod_del_pos = requests
		.iter()
		.position(|r| r.starts_with("DELETE") && r.contains("/pods/proj"))
		.expect("down must DELETE the pod");
	let container_del_positions: Vec<usize> = requests
		.iter()
		.enumerate()
		.filter(|(_, r)| r.starts_with("DELETE") && r.contains("/containers/"))
		.map(|(i, _)| i)
		.collect();
	assert!(
		!container_del_positions.is_empty(),
		"down must delete at least one container; requests: {requests:?}"
	);
	assert!(
		container_del_positions.iter().all(|p| *p < pod_del_pos),
		"every container DELETE must happen before the pod DELETE; requests: {requests:?}"
	);
}
