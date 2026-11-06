//! A watch restart gives the container the stop grace its service declares.

use crate::engine::fake_podman::{self, FakeReply};
use crate::engine::Engine;

fn file(grace: &str) -> crate::compose::types::ComposeFile {
	crate::parse_str(&format!("services:\n  web:\n    image: x\n{grace}")).unwrap()
}

#[tokio::test]
async fn the_grace_is_the_one_the_service_declares_or_the_default() {
	let fake = fake_podman::start(|_, _| (404, String::new()));
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	assert_eq!(
		engine.watch_grace(&file("    stop_grace_period: 30s\n"), "web"),
		30
	);
	assert_eq!(
		engine.watch_grace(&file("    stop_grace_period: 1500ms\n"), "web"),
		2
	);
	assert_eq!(engine.watch_grace(&file(""), "web"), 10);
	assert_eq!(engine.watch_grace(&file(""), "gone"), 10);
}

#[tokio::test]
async fn the_restart_request_carries_that_grace_and_not_five_seconds() {
	let fake = fake_podman::start_replying(|method, target| {
		if method == "POST" && target.contains("/restart?") {
			FakeReply::Body(204, String::new())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	for grace in [30, 0, 10] {
		engine.watch_restart("proj-web-1", grace).await.unwrap();
		let seen = fake.requests.lock().unwrap().clone();
		let last = seen.last().unwrap();
		assert!(
			last.contains(&format!("proj-web-1/restart?timeout={grace}")),
			"{last}"
		);
	}
}

// The dispatch, where the grace is decided: a rule of the file, an event, and the
// request that leaves for the daemon.

fn rule_file(action: &str) -> crate::compose::types::ComposeFile {
	crate::parse_str(&format!(
		"services:\n  web:\n    image: x\n    stop_grace_period: 30s\n    develop:\n      watch:\n        - path: src\n          action: {action}\n"
	))
	.unwrap()
}

fn entry(file: &crate::compose::types::ComposeFile) -> super::super::RuleEntry {
	super::super::RuleEntry {
		service_name: "web".into(),
		container_name: "proj-web-1".into(),
		rule: file.services["web"].develop.as_ref().unwrap().watch[0].clone(),
		abs_path: std::path::PathBuf::from("/p/src"),
		build_context_abs: None,
		build_context_patterns: vec![],
		mounts: std::sync::Arc::new(vec![]),
		shape: super::super::placement::RuleShape::Directory,
	}
}

async fn dispatched(action: &str, timeout: Option<i32>) -> String {
	let fake = fake_podman::start_replying(|method, target| {
		if method == "POST" && target.contains("/restart?") {
			FakeReply::Body(204, String::new())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
		.with_stop_timeout(timeout);
	let file = rule_file(action);
	let mut ensured = std::collections::HashSet::new();
	engine
		.dispatch_action(
			&file,
			std::path::Path::new("/p/src/a.txt"),
			&entry(&file),
			&mut ensured,
		)
		.await
		.unwrap();
	let seen = fake.requests.lock().unwrap().clone();
	seen.into_iter()
		.find(|r| r.contains("/restart?"))
		.expect("a restart was sent")
}

#[tokio::test]
async fn a_rule_restart_leaves_with_the_grace_of_its_service() {
	for action in ["restart", "sync+restart"] {
		let request = dispatched(action, None).await;
		assert!(
			request.contains("restart?timeout=30"),
			"{action}: {request}"
		);
	}
}

#[tokio::test]
async fn the_timeout_of_the_command_wins_over_the_service() {
	for action in ["restart", "sync+restart"] {
		let request = dispatched(action, Some(60)).await;
		assert!(
			request.contains("restart?timeout=60"),
			"{action}: {request}"
		);
	}
}
