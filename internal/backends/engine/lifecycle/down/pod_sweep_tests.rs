//! `down` removes the project's pod even when the file no longer declares one.
//!
//! The pod was removed only when the file still carried `x-podman-pod`. Drop
//! the key between `up` and `down` and the pod and its infra container stayed
//! running, which kept the project network in use, so the network removal
//! failed and `down` exited 1.

use crate::engine::fake_podman;
use crate::engine::Engine;

fn engine() -> (fake_podman::FakePodman, Engine) {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/pods/json") {
			(200, r#"[{"Name":"proj"}]"#.to_string())
		} else if method == "GET" && target.contains("/pods/proj/json") {
			(200, r#"{"Labels":{"podup.project":"proj"}}"#.to_string())
		} else if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "DELETE" {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	(fake, engine)
}

fn pod_deletes(fake: &fake_podman::FakePodman) -> Vec<String> {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.starts_with("DELETE ") && r.contains("/pods/"))
		.cloned()
		.collect()
}

#[tokio::test]
async fn a_file_without_pod_mode_still_gets_the_project_pod_removed() {
	let (fake, engine) = engine();
	let file = crate::parse_str("services:\n  web:\n    image: x\n").unwrap();
	engine.down(&file).await.unwrap();
	let removed = pod_deletes(&fake);
	assert_eq!(removed.len(), 1, "{removed:?}");
	assert!(removed[0].contains("/pods/proj"), "{removed:?}");
}

#[tokio::test]
async fn a_file_with_pod_mode_removes_the_pod_once() {
	let (fake, engine) = engine();
	let file = crate::parse_str("x-podman-pod: true\nservices:\n  web:\n    image: x\n").unwrap();
	engine.down(&file).await.unwrap();
	let removed = pod_deletes(&fake);
	assert_eq!(removed.len(), 1, "{removed:?}");
}
