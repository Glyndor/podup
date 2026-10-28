//! What a changed `env_file:` does to a container that is already up.
//!
//! `up` compares the `podup.config-hash` label of an existing container with
//! the hash it computes now. An equal hash means "unchanged", and the
//! container is left running exactly as it was. The service definition only
//! carries the *name* of the `env_file:` file, so an edit to the file's
//! contents left the hash equal and the container running with the
//! environment it had before the edit.
//!
//! These tests drive a whole `up` against the recording fake: the fake
//! answers the container list, the image inspect and the creates, and the test
//! publishes what the first `up` created so the second one has something to
//! compare against.

use std::sync::{Arc, Mutex};

use crate::compose::types::ComposeFile;
use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;

const CURRENT: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/// The containers the fake host reports as running. Shared with the test so it
/// can publish what the previous `up` created.
type Running = Arc<Mutex<String>>;

/// A host that answers what `up` asks: the project's container list, the image
/// inspect, and the creates, starts and deletes.
fn host(running: Running) -> FakePodman {
	fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			return (200, running.lock().unwrap().clone());
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

/// The container list of a project whose single service is already up,
/// carrying the config hash the previous `up` recorded on the label.
fn running_list(hash: &str) -> String {
	serde_json::Value::Array(vec![serde_json::json!({
		"Id": "proj-web-1",
		"Names": ["proj-web-1"],
		"ImageID": CURRENT,
		"Labels": {
			"podup.project": "proj",
			"podup.service": "web",
			"podup.config-hash": hash,
		},
	})])
	.to_string()
}

/// The `podup.config-hash` label of the last container the fake was asked to
/// create, read off the create payload rather than recomputed, so the test
/// never asserts against its own idea of the hash.
fn created_hash(fake: &FakePodman) -> String {
	let requests = fake.requests.lock().unwrap();
	let bodies = fake.bodies.lock().unwrap();
	let index = requests
		.iter()
		.rposition(|r| r.contains("/containers/create"))
		.expect("the up must have created a container");
	let body: serde_json::Value =
		serde_json::from_slice(&bodies[index]).expect("the create payload is JSON");
	body["labels"]["podup.config-hash"]
		.as_str()
		.expect("the create payload carries the config hash label")
		.to_string()
}

/// The requests the fake answered whose line contains `needle`.
fn seen(fake: &FakePodman, needle: &str) -> Vec<String> {
	let requests = fake.requests.lock().unwrap();
	requests
		.iter()
		.filter(|r| r.contains(needle))
		.cloned()
		.collect()
}

async fn up(
	fake: &FakePodman,
	file: &ComposeFile,
	base: &std::path::Path,
) -> crate::error::Result<()> {
	Engine::with_base_dir(fake.client(), "proj".into(), base.to_path_buf())
		.up_with_options(file, false, &[], &[], false, false, false, false)
		.await
}

/// The compose file under test: one service whose whole environment comes
/// from a file.
fn compose() -> ComposeFile {
	crate::parse_str(
		"services:\n  web:\n    image: nginx:1.27\n    pull_policy: never\n    env_file: [app.env]\n",
	)
	.expect("the compose file parses")
}

/// An edited `env_file:` is a changed environment, so the container that
/// carries the old one is replaced instead of being left running.
#[tokio::test]
async fn a_changed_env_file_recreates_the_container() {
	let base = tempfile::tempdir().expect("tempdir");
	std::fs::write(base.path().join("app.env"), "COLOR=azul\n").expect("write the env file");
	let file = compose();
	let running = Running::new(Mutex::new("[]".to_string()));
	let fake = host(running.clone());

	up(&fake, &file, base.path())
		.await
		.expect("the first up must succeed");
	assert_eq!(
		seen(&fake, "/containers/create").len(),
		1,
		"the first up creates the container"
	);

	// Publish what the first `up` created, so the second one finds a container
	// to compare against instead of creating a second one from scratch.
	let hash = created_hash(&fake);
	*running.lock().unwrap() = running_list(&hash);

	// The environment changes; the service definition does not.
	std::fs::write(base.path().join("app.env"), "COLOR=rojo\n").expect("rewrite the env file");

	up(&fake, &file, base.path())
		.await
		.expect("the second up must succeed");
	assert_eq!(
		seen(&fake, "/containers/create").len(),
		2,
		"a changed env file must replace the container that carries the old environment"
	);
}

/// The control on the test above: with the environment untouched, the second
/// `up` finds the container unchanged and leaves it alone. Without this, a
/// test that recreated every container on every `up` would also pass.
#[tokio::test]
async fn an_unchanged_env_file_leaves_the_container_alone() {
	let base = tempfile::tempdir().expect("tempdir");
	std::fs::write(base.path().join("app.env"), "COLOR=azul\n").expect("write the env file");
	let file = compose();
	let running = Running::new(Mutex::new("[]".to_string()));
	let fake = host(running.clone());

	up(&fake, &file, base.path())
		.await
		.expect("the first up must succeed");
	let hash = created_hash(&fake);
	*running.lock().unwrap() = running_list(&hash);

	up(&fake, &file, base.path())
		.await
		.expect("the second up must succeed");
	assert_eq!(
		seen(&fake, "/containers/create").len(),
		1,
		"an unchanged env file must leave the container alone"
	);
}
