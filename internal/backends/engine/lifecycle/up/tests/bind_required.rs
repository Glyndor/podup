//! A bind mount with `create_host_path: false` and a missing host path stops
//! `up` at that service with an error naming the service and the path, and no
//! container is created for it.

#![cfg(unix)]

use crate::engine::fake_podman;
use crate::engine::Engine;

#[tokio::test]
async fn up_fails_naming_service_and_path_when_a_required_bind_source_is_missing() {
	let dir = tempfile::tempdir().unwrap();
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "POST" && target.contains("/networks/create") {
			(200, "{}".to_string())
		} else {
			(200, String::new())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "probe".into(), dir.path().to_path_buf());
	let mut file = crate::parse_str(
		"services:\n  web:\n    image: alpine:latest\n    volumes:\n      - type: bind\n        \
		 source: ./absent\n        target: /data\n        bind:\n          create_host_path: false\n",
	)
	.expect("fixture must parse");
	crate::compose::normalize_default_network(&mut file);

	let err = engine
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect_err("a missing required bind source must fail up");
	let msg = err.to_string();
	let missing = dir.path().join("absent");
	assert!(msg.contains("\"web\""), "service not named: {msg}");
	assert!(
		msg.contains(&missing.to_string_lossy().into_owned()),
		"path not named: {msg}"
	);
	assert!(!missing.exists(), "the source must not be created");
	let created: Vec<String> = fake
		.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.contains("/containers/create"))
		.cloned()
		.collect();
	assert!(
		created.is_empty(),
		"no container may be created: {created:?}"
	);
}
