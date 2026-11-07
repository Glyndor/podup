//! A watch rule acts on every replica of its service, not only the first.

use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::fake_podman;
use crate::engine::Engine;

use super::super::placement::RuleShape;
use super::super::RuleEntry;

fn fake() -> fake_podman::FakePodman {
	fake_podman::start(|method, target| {
		if method == "POST" && target.ends_with("/containers/create") {
			(200, r#"{"Id":"container-1"}"#.to_string())
		} else if (method == "POST" && target.ends_with("/start")) || method == "DELETE" {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	})
}

fn file(scale: u32) -> crate::compose::types::ComposeFile {
	crate::parse_str(&format!(
		"services:\n  web:\n    image: alpine:latest\n    scale: {scale}\n    develop:\n      watch:\n        - path: src\n          action: restart\n"
	))
	.unwrap()
}

fn template(file: &crate::compose::types::ComposeFile) -> RuleEntry {
	let rule = file.services["web"].develop.as_ref().unwrap().watch[0].clone();
	RuleEntry {
		service_name: "web".into(),
		container_name: String::new(),
		rule,
		abs_path: PathBuf::from("/p/src"),
		build_context_abs: None,
		build_context_patterns: vec![],
		mounts: Arc::new(vec![]),
		shape: RuleShape::Directory,
	}
}

fn engine(fake: &fake_podman::FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

#[tokio::test]
async fn a_rule_has_one_entry_per_replica() {
	let fake = fake();
	let file = file(3);
	let entries = engine(&fake).per_replica(template(&file), &file.services["web"]);
	let names: Vec<_> = entries.iter().map(|e| e.container_name.as_str()).collect();
	assert_eq!(names, ["proj-web-1", "proj-web-2", "proj-web-3"]);
}

#[tokio::test]
async fn a_service_scaled_to_zero_keeps_the_one_name_it_always_had() {
	let fake = fake();
	let file = file(0);
	let entries = engine(&fake).per_replica(template(&file), &file.services["web"]);
	assert_eq!(entries.len(), 1);
}

#[tokio::test]
async fn a_rebuild_recreates_every_replica_in_index_order() {
	let fake = fake();
	let engine = engine(&fake);
	engine.watch_rebuild(&file(3), "web").await.unwrap();
	let seen = fake.requests.lock().unwrap().clone();
	let creates = seen
		.iter()
		.filter(|r| r.ends_with("/containers/create"))
		.count();
	assert_eq!(creates, 3, "{seen:?}");
	let starts: Vec<_> = seen.iter().filter(|r| r.ends_with("/start")).collect();
	let order: Vec<_> = ["proj-web-1", "proj-web-2", "proj-web-3"]
		.iter()
		.map(|n| starts.iter().position(|r| r.contains(n)))
		.collect();
	assert!(order.windows(2).all(|w| w[0] < w[1]), "{starts:?}");
}

#[tokio::test]
async fn a_rebuild_that_fails_stops_before_the_next_replica() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.ends_with("/containers/create") {
			(500, r#"{"message":"create refused"}"#.to_string())
		} else if method == "DELETE" {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = engine(&fake);
	assert!(engine.watch_rebuild(&file(3), "web").await.is_err());
	let seen = fake.requests.lock().unwrap().clone();
	let creates = seen
		.iter()
		.filter(|r| r.ends_with("/containers/create"))
		.count();
	assert_eq!(creates, 1, "the first failure stops the walk: {seen:?}");
}
