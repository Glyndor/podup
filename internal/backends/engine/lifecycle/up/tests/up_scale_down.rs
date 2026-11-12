//! Lowering the replica count in the compose file removes the surplus.
//!
//! Only a `--scale` override used to trigger the reconciliation, so going from
//! three replicas to `scale: 1` in the file left replicas two and three
//! running forever.

use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;

const CURRENT: &str = "aaaa";

/// A host whose project already runs `web` as three replicas.
fn host() -> FakePodman {
	let running = serde_json::Value::Array(
		(1..=3)
			.map(|n| {
				serde_json::json!({
					"Id": format!("proj-web-{n}"),
					"Names": [format!("proj-web-{n}")],
					"ImageID": CURRENT,
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
			return (200, format!(r#"{{"Id":"{CURRENT}"}}"#));
		}
		match method {
			"POST" => (200, "{}".to_string()),
			"DELETE" => (200, String::new()),
			_ => (404, r#"{"message":"not found"}"#.to_string()),
		}
	})
}

/// `up()` reads `PODUP_MAX_REPLICAS`, and `replica_limit_default_and_env_override`
/// sets it to `2` for a while: a three-replica fixture inside `up()` at that
/// moment fails with `ReplicaLimitExceeded`. The lock is the one that test and
/// `up_one.rs` already take. Held in a struct, as `up_one.rs` does, because the
/// test has to keep it across the `await` of `up()`.
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

async fn up(fake: &FakePodman, yaml: &str) {
	let _lock = replica_lock();
	let file = crate::parse_str(yaml).expect("the fixture parses");
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect("up succeeds");
}

fn removed(fake: &FakePodman, name: &str) -> bool {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.any(|r| r.starts_with("DELETE") && r.contains(&format!("/containers/{name}")))
}

#[tokio::test]
async fn a_lower_scale_in_the_file_removes_the_surplus_replicas() {
	let fake = host();
	up(&fake, "services:\n  web:\n    image: x\n    scale: 1\n").await;
	assert!(removed(&fake, "proj-web-2"), "replica two must be removed");
	assert!(
		removed(&fake, "proj-web-3"),
		"replica three must be removed"
	);
	assert!(!removed(&fake, "proj-web-1"), "replica one is wanted");
}

#[tokio::test]
async fn a_lower_deploy_replicas_in_the_file_removes_the_surplus_replicas() {
	let fake = host();
	up(
		&fake,
		"services:\n  web:\n    image: x\n    deploy:\n      replicas: 2\n",
	)
	.await;
	assert!(
		removed(&fake, "proj-web-3"),
		"replica three must be removed"
	);
	assert!(!removed(&fake, "proj-web-2"), "replica two is wanted");
}

#[tokio::test]
async fn a_file_that_still_wants_every_replica_removes_none() {
	let fake = host();
	up(&fake, "services:\n  web:\n    image: x\n    scale: 3\n").await;
	for name in ["proj-web-1", "proj-web-2", "proj-web-3"] {
		assert!(!removed(&fake, name), "{name} is wanted");
	}
}
