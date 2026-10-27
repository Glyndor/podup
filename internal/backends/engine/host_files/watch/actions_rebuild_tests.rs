//! A watch rebuild recreates the container the way `up` does.
//!
//! `up` runs the service's `pre_start` steps before the container exists and
//! its `post_start` hooks inside the container once it is up. The rebuild
//! from `watch` created the container on its own, so a service that needs
//! either step came up without it.

use crate::engine::fake_podman;
use crate::engine::Engine;

/// A fake that answers the container and exec round trips a rebuild needs,
/// and records every request in the order it arrived.
fn fake_podman() -> fake_podman::FakePodman {
	fake_podman::start(|method, target| {
		if method == "POST" && target.ends_with("/exec") {
			(200, r#"{"Id":"exec-1"}"#.to_string())
		} else if method == "POST" && target.ends_with("/containers/create") {
			(200, r#"{"Id":"container-1"}"#.to_string())
		} else if method == "POST" && target.ends_with("/start") {
			(200, String::new())
		} else if method == "POST" && target.contains("/wait") {
			(200, "0".to_string())
		} else if method == "GET" && target.contains("/exec/") {
			(200, r#"{"ExitCode":0}"#.to_string())
		} else if method == "DELETE" {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	})
}

const FILE: &str = "services:\n  web:\n    image: alpine:latest\n    pre_start:\n      - command: [\"sh\", \"-c\", \"echo init\"]\n    post_start:\n      - command: [\"sh\", \"-c\", \"echo ready\"]\n";

/// The name of the container every `POST /containers/create` asked for, in
/// the order the requests arrived.
fn created_names(fake: &fake_podman::FakePodman) -> Vec<String> {
	let requests = fake.requests.lock().unwrap().clone();
	let bodies = fake.bodies.lock().unwrap().clone();
	requests
		.iter()
		.enumerate()
		.filter(|(_, r)| r.ends_with("/containers/create"))
		.map(|(i, _)| {
			let spec: serde_json::Value = serde_json::from_slice(&bodies[i]).unwrap_or_default();
			spec.get("name")
				.and_then(|n| n.as_str())
				.unwrap_or_default()
				.to_string()
		})
		.collect()
}

/// The argv of every exec the fake was asked to run.
fn exec_argvs(fake: &fake_podman::FakePodman) -> Vec<Vec<String>> {
	fake.bodies
		.lock()
		.unwrap()
		.iter()
		.filter_map(|body| {
			let value: serde_json::Value = serde_json::from_slice(body).ok()?;
			value
				.get("Cmd")
				.and_then(|cmd| serde_json::from_value(cmd.clone()).ok())
		})
		.collect()
}

/// The rebuild runs the `pre_start` step before the service container
/// exists, and the `post_start` hook inside the container once it is up.
#[tokio::test]
async fn a_watch_rebuild_runs_the_pre_start_step_and_the_post_start_hook() {
	let fake = fake_podman();
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	let file = crate::parse_str(FILE).unwrap();

	engine.watch_rebuild(&file, "web").await.unwrap();

	let created = created_names(&fake);
	assert_eq!(
		created.len(),
		2,
		"the pre_start step and the service container must both be created: {created:?}"
	);
	let pre_start = created
		.iter()
		.position(|name| name.contains("pre-start"))
		.unwrap_or_else(|| panic!("the pre_start step container was never created: {created:?}"));
	let service = created
		.iter()
		.position(|name| name == "proj-web-1")
		.unwrap_or_else(|| panic!("the service container was never created: {created:?}"));
	assert!(
		pre_start < service,
		"the pre_start step runs before the service container exists: {created:?}"
	);

	let argvs = exec_argvs(&fake);
	assert!(
		argvs.iter().any(|argv| argv == &["sh", "-c", "echo ready"]),
		"the post_start hook must run inside the container: {argvs:?}"
	);
}
