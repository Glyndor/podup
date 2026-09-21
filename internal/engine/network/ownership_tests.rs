//! Network ownership: a `up` that hits a 409 must not silently adopt another
//! project's network, and a `down` must not delete one either.
//!
//! Each test wires a fake libpod socket, drives `Engine::create_networks` /
//! `Engine::down_with_options`, and asserts the refusal shape. The two halves
//! (`create_networks` and `down_with_options`) are exercised separately so a
//! failure points at the right half.

use super::*;
use crate::compose::types::NetworkConfig;
#[cfg(unix)]
use crate::engine::fake_podman;
use crate::error::ComposeError;
use crate::parse_str;

#[cfg(unix)]
fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

#[cfg(unix)]
fn file_with_named_network(project_label: &str, project_key: &str, name: &str) -> ComposeFile {
	let yaml = format!(
		"networks:\n  {project_label}:\n    name: {name}\nservices:\n  web:\n    image: alpine\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let mut file = parse_str(&yaml).expect("parse fixture");
	// Rename the network key so two engines with different projects do not
	// collide on the compose-file key. The `name:` field is what matters for
	// the ownership check.
	file.networks.clear();
	file.networks.insert(
		project_key.to_string(),
		Some(NetworkConfig {
			name: Some(name.to_string()),
			..Default::default()
		}),
	);
	// Force the default service to use that single network so `up_resources`
	// does not surface a `default` row that the create stage would never
	// produce a name for. `create_networks` only iterates `file.networks`,
	// so the unused-service / unused-network pair is fine for this test;
	// `up` itself walks more.
	file
}

#[cfg(unix)]
fn file_with_named_external(project_label: &str, name: &str) -> ComposeFile {
	let mut file = file_with_named_network(project_label, project_label, name);
	file.networks.insert(
		project_label.to_string(),
		Some(NetworkConfig {
			name: Some(name.to_string()),
			external: Some(true),
			..Default::default()
		}),
	);
	file
}

/// `create_networks` is the only path the integration suite exercises through
/// `up`, but the test goes direct: `up` pulls images, builds boards and
/// starts containers, none of which the ownership question depends on.
#[cfg(unix)]
async fn run_create(engine: &Engine, file: &ComposeFile) -> Result<()> {
	engine.create_networks(file).await
}

#[cfg(unix)]
async fn run_down(engine: &Engine, file: &ComposeFile) -> Result<()> {
	engine.down_with_options(file, false).await
}

// ---------------------------------------------------------------------------
// create_networks: foreign-labelled network is refused
// ---------------------------------------------------------------------------

/// A second project's `up` against an existing network labelled for the
/// first project must fail with an error that names the contested physical
/// network, the owner and this project, and points at `external: true`.
/// No DELETE ever fires: a 409 on create is the gate.
#[cfg(unix)]
#[tokio::test]
async fn create_refuses_a_network_labelled_for_a_different_project() {
	let shared = "shared-fk-a";
	let fake = fake_podman::start(move |method, target| {
		if method == "POST" && target.ends_with("/networks/create") {
			(409, r#"{"message":"network already exists"}"#.to_string())
		} else if method == "GET" && target.contains("/networks/") && target.ends_with("/json") {
			// The existing network belongs to a different project.
			(
				200,
				format!(r#"{{"name":"{shared}","labels":{{"podup.project":"owner-fk-a"}}}}"#),
			)
		} else {
			(404, r#"{"message":"not used"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "intruder-fk-a");
	let file = file_with_named_network("n1", "n1", shared);

	let err = run_create(&e, &file)
		.await
		.expect_err("foreign-labelled network must refuse the create");
	let msg = format!("{err:?}");
	assert!(
		matches!(err, ComposeError::Unsupported(_)),
		"expected Unsupported, got {err:?}"
	);
	assert!(msg.contains(shared), "error names the network: {msg}");
	assert!(msg.contains("owner-fk-a"), "error names the owner: {msg}");
	assert!(
		msg.contains("intruder-fk-a"),
		"error names the intruder project: {msg}"
	);
	assert!(
		msg.contains("external"),
		"error points at external: true: {msg}"
	);

	// No DELETE reached the fake: the create was the only place that would
	// have caused harm, and it was refused before any destructive call.
	let seen = fake.requests.lock().unwrap();
	assert!(
		!seen.iter().any(|r| r.starts_with("DELETE")),
		"refusal must not delete: {seen:?}"
	);
}

// ---------------------------------------------------------------------------
// create_networks: unlabelled network is refused (the chosen policy)
// ---------------------------------------------------------------------------

/// An existing network with no `podup.project` label is treated the same as
/// a foreign-labelled one: refused, with `external: true` named as the
/// escape hatch. The label is the only ownership evidence podup writes;
/// "no one owns it" and "another stack already claimed it" are
/// indistinguishable without it.
#[cfg(unix)]
#[tokio::test]
async fn create_refuses_an_unlabelled_existing_network() {
	let shared = "shared-fk-b";
	let fake = fake_podman::start(move |method, target| {
		if method == "POST" && target.ends_with("/networks/create") {
			(409, r#"{"message":"network already exists"}"#.to_string())
		} else if method == "GET" && target.contains("/networks/") && target.ends_with("/json") {
			(200, format!(r#"{{"name":"{shared}","labels":{{}}}}"#))
		} else {
			(404, r#"{"message":"not used"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "intruder-fk-b");
	let file = file_with_named_network("n1", "n1", shared);

	let err = run_create(&e, &file)
		.await
		.expect_err("unlabelled existing network must refuse");
	let msg = format!("{err:?}");
	assert!(
		matches!(err, ComposeError::Unsupported(_)),
		"expected Unsupported, got {err:?}"
	);
	assert!(msg.contains(shared), "error names the network: {msg}");
	assert!(
		msg.contains("no podup.project"),
		"error explains the unlabelled case: {msg}"
	);
	assert!(
		msg.contains("external"),
		"error points at external: true: {msg}"
	);
}

// ---------------------------------------------------------------------------
// create_networks: external: true skips the ownership check entirely
// ---------------------------------------------------------------------------

/// `external: true` is the escape hatch: `ensure_external_exists` verifies
/// the network is on the host and the ownership check never fires. The
/// 409 from create never reaches this path because external networks are
/// verified, not created.
#[cfg(unix)]
#[tokio::test]
async fn create_external_network_is_verified_not_owned() {
	let shared = "shared-fk-c";
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/networks/") && target.ends_with("/json") {
			// Whatever the labels say: `external: true` makes them
			// irrelevant.
			(
				200,
				format!(r#"{{"name":"{shared}","labels":{{"podup.project":"owner-fk-c"}}}}"#),
			)
		} else {
			(404, r#"{"message":"not used"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "intruder-fk-c");
	let file = file_with_named_external("n1", shared);

	run_create(&e, &file)
		.await
		.expect("external: true must not run the ownership check");

	// The only call that should have reached the fake is the verify-exists
	// GET. A POST to /networks/create on an external path is the bug.
	let seen = fake.requests.lock().unwrap();
	assert!(
		!seen
			.iter()
			.any(|r| r.starts_with("POST") && r.contains("/networks/create")),
		"external network must not be created: {seen:?}"
	);
	assert!(
		seen.iter()
			.any(|r| r.starts_with("GET") && r.contains(&format!("/networks/{shared}/json"))),
		"external network must be verified: {seen:?}"
	);
}

// ---------------------------------------------------------------------------
// create_networks: a network labelled for us is accepted (idempotent re-up)
// ---------------------------------------------------------------------------

/// The existing 409→Exists path: a network already labelled for this
/// project is accepted as the idempotent re-`up` shape, no error.
#[cfg(unix)]
#[tokio::test]
async fn create_accepts_a_network_already_labelled_for_this_project() {
	let shared = "shared-fk-d";
	let fake = fake_podman::start(move |method, target| {
		if method == "POST" && target.ends_with("/networks/create") {
			(409, r#"{"message":"network already exists"}"#.to_string())
		} else if method == "GET" && target.contains("/networks/") && target.ends_with("/json") {
			(
				200,
				format!(r#"{{"name":"{shared}","labels":{{"podup.project":"intruder-fk-d"}}}}"#),
			)
		} else {
			(404, r#"{"message":"not used"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "intruder-fk-d");
	let file = file_with_named_network("n1", "n1", shared);

	run_create(&e, &file)
		.await
		.expect("a network already labelled for this project must be accepted");
}

// ---------------------------------------------------------------------------
// down_with_options: refuses to remove a foreign-labelled network
// ---------------------------------------------------------------------------

/// The half that loses data. A compose file declaring `name: <shared>` and
/// no containers on the host tries to remove `<shared>`; libpod would
/// happily honour the DELETE because no container is attached; the guard
/// refuses before the request lands.
#[cfg(unix)]
#[tokio::test]
async fn down_refuses_to_remove_a_network_labelled_for_a_different_project() {
	let shared = "shared-fk-e";
	let fake = fake_podman::start(move |method, target| {
		// No live containers for this project; libpod would happily DELETE.
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "GET" && target.contains("/networks/") && target.ends_with("/json") {
			(
				200,
				format!(r#"{{"name":"{shared}","labels":{{"podup.project":"owner-fk-e"}}}}"#),
			)
		} else {
			(404, r#"{"message":"not used"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "intruder-fk-e");
	let file = file_with_named_network("n1", "n1", shared);

	let err = run_down(&e, &file)
		.await
		.expect_err("down must refuse a foreign-labelled network");
	let msg = format!("{err:?}");
	assert!(
		matches!(err, ComposeError::Unsupported(_)),
		"expected Unsupported, got {err:?}"
	);
	assert!(msg.contains(shared), "error names the network: {msg}");
	assert!(msg.contains("owner-fk-e"), "error names the owner: {msg}");

	let seen = fake.requests.lock().unwrap();
	assert!(
		!seen
			.iter()
			.any(|r| r.starts_with("DELETE") && r.contains("/networks/")),
		"no DELETE must reach the fake: {seen:?}"
	);
}

// ---------------------------------------------------------------------------
// down_with_options: a network labelled for us is removed as before
// ---------------------------------------------------------------------------

/// The pre-fix happy path: a network labelled for this project is
/// removed by name, no refusal.
#[cfg(unix)]
#[tokio::test]
async fn down_removes_a_network_labelled_for_this_project() {
	let shared = "shared-fk-f";
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "GET" && target.contains("/networks/") && target.ends_with("/json") {
			(
				200,
				format!(r#"{{"name":"{shared}","labels":{{"podup.project":"intruder-fk-f"}}}}"#),
			)
		} else if method == "DELETE" && target.contains(&format!("/networks/{shared}")) {
			(200, String::new())
		} else {
			(404, r#"{"message":"not used"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "intruder-fk-f");
	let file = file_with_named_network("n1", "n1", shared);

	run_down(&e, &file)
		.await
		.expect("a network labelled for this project must be removed");

	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.any(|r| r.starts_with("DELETE") && r.contains(&format!("/networks/{shared}"))),
		"the labelled-for-us network must be removed: {seen:?}"
	);
}
