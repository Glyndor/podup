//! The client gate on its own: an `up` that reaches the engine with a dry-run
//! client sends nothing that is not a read, whatever the command line did.
//!
//! The CLI refuses every mutating command before it gets here, so this is the
//! second wall: if a command were ever allowed by mistake, or a path were added
//! that nobody guarded, the requests still would not leave the client.

use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;

/// A host whose project runs `web` as three replicas, so an `up` of a smaller
/// file has containers to remove and the images to inspect.
fn host() -> FakePodman {
	let running = serde_json::Value::Array(
		(1..=3)
			.map(|n| {
				serde_json::json!({
					"Id": format!("proj-web-{n}"),
					"Names": [format!("proj-web-{n}")],
					"ImageID": "aaaa",
					"Labels": {"podup.project": "proj", "podup.service": "web"},
				})
			})
			.collect(),
	)
	.to_string();
	fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			return (200, running.clone());
		}
		if method == "GET" && target.contains("/images/") && target.ends_with("/json") {
			return (200, r#"{"Id":"aaaa"}"#.to_string());
		}
		match method {
			"POST" => (200, "{}".to_string()),
			"DELETE" => (200, String::new()),
			_ => (404, r#"{"message":"not found"}"#.to_string()),
		}
	})
}

/// `up()` reads `PODUP_MAX_REPLICAS`, which another test sets to `2` for a while,
/// so a three-replica fixture takes the same lock they do. The guard is held in a
/// struct, as `up_scale_down.rs` does, because the test keeps it across `await`.
struct ReplicaLock {
	_lock: std::sync::MutexGuard<'static, ()>,
}

fn replica_lock() -> ReplicaLock {
	ReplicaLock {
		_lock: crate::engine::lifecycle::scale::scale_tests::MAX_REPLICAS_TEST_LOCK
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner()),
	}
}

fn mutations(fake: &FakePodman) -> Vec<String> {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|request| !request.starts_with("GET") && !request.starts_with("HEAD"))
		.cloned()
		.collect()
}

#[tokio::test]
async fn an_up_through_a_dry_run_client_sends_no_mutating_request() {
	let _lock = replica_lock();
	let fake = host();
	let file = crate::parse_str("services:\n  web:\n    image: x\n    scale: 1\n")
		.expect("the fixture parses");
	let result = Engine::with_base_dir(
		fake.client().with_dry_run(true),
		"proj".into(),
		std::env::temp_dir(),
	)
	.up_with_options(&file, false, &[], &[], false, false, false, false)
	.await;
	assert!(
		mutations(&fake).is_empty(),
		"a dry-run client let a request through: {:?}",
		mutations(&fake)
	);
	assert!(
		result.is_err(),
		"up has surplus replicas to remove, so it must stop at the first refusal"
	);
}

#[tokio::test]
async fn the_same_up_through_a_normal_client_does_mutate() {
	// The control: without the gate the fixture really produces DELETEs, so the
	// test above is not passing because there was nothing to send.
	let _lock = replica_lock();
	let fake = host();
	let file = crate::parse_str("services:\n  web:\n    image: x\n    scale: 1\n")
		.expect("the fixture parses");
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect("up succeeds");
	assert!(!mutations(&fake).is_empty());
}
