use super::container_rm_path;

#[cfg(unix)]
use super::Engine;
#[cfg(unix)]
use crate::compose::types::{ComposeFile, Service};
#[cfg(unix)]
use crate::engine::fake_podman;
#[cfg(unix)]
use crate::error::ComposeError;

#[cfg(unix)]
fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

/// #598: a repeated `up` finding a stopped-but-unchanged container must not
/// silently succeed when the start genuinely fails (e.g. its published host
/// port is now taken by something else).
#[tokio::test]
#[cfg(unix)]
async fn ensure_started_propagates_a_real_start_failure() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.contains("/proj-web-1/start") {
			(500, r#"{"message":"address already in use"}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let err = e
		.ensure_started("proj-web-1")
		.await
		.expect_err("a real start failure must propagate, not exit 0 silently");
	match err {
		ComposeError::Podman(pe) => assert!(pe.is_status(500), "got {pe}"),
		other => panic!("expected a Podman error, got {other:?}"),
	}
	assert!(
		fake.requests
			.lock()
			.unwrap()
			.iter()
			.any(|r| r.contains("/proj-web-1/start")),
		"expected the fake socket to have received the start request"
	);
}

/// A container that vanished between the presence check and the start
/// (or one Podman reports as already running, 304) is an idempotent no-op,
/// matching `run_lifecycle_op`.
#[tokio::test]
#[cfg(unix)]
async fn ensure_started_tolerates_404_and_304() {
	let fake = fake_podman::start(|_, _| (404, r#"{"message":"no such container"}"#.to_string()));
	let e = engine_with(fake.client(), "proj");
	e.ensure_started("proj-web-1")
		.await
		.expect("404 must be an idempotent no-op");

	let fake = fake_podman::start(|_, _| (304, String::new()));
	let e = engine_with(fake.client(), "proj");
	e.ensure_started("proj-web-1")
		.await
		.expect("304 must be an idempotent no-op");
}

/// Two containers to tear down: one whose removal genuinely fails (a busy
/// mount, an active exec session), one that removes cleanly. `down` must
/// still attempt (and complete) the second before exiting non-zero for the
/// first (#598) — a CI teardown must not be told it succeeded.
#[tokio::test]
#[cfg(unix)]
async fn down_propagates_a_real_removal_failure_after_completing_the_rest() {
	let containers = r#"[
		{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-db-1"],"Labels":{"podup.service":"db"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else if method == "POST" && target.contains("/stop") {
			(200, String::new())
		} else if method == "DELETE" && target.contains("/proj-web-1?force=true") {
			(500, r#"{"message":"device or resource busy"}"#.to_string())
		} else if method == "DELETE" && target.contains("/proj-db-1?force=true") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());
	file.services.insert("db".into(), Service::default());

	let err = e
		.down_with_options(&file, false)
		.await
		.expect_err("a real container-removal failure must propagate");
	assert!(
		matches!(err, ComposeError::Podman(ref pe) if pe.is_status(500)),
		"got {err:?}"
	);

	// Best-effort: the healthy container must still have been reached even
	// though the other one failed.
	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.any(|r| r.contains("DELETE") && r.contains("/proj-db-1?force=true")),
		"expected proj-db-1 to be removed despite proj-web-1 failing: {seen:?}"
	);
}

/// A second `down` on an already torn-down project (no live containers,
/// nothing left to sweep) must still exit 0 — idempotency is preserved.
#[tokio::test]
#[cfg(unix)]
async fn down_on_an_already_torn_down_project_is_still_ok() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	e.down_with_options(&file, false)
		.await
		.expect("a re-run down on a torn-down project must still exit 0");
}

#[test]
fn rm_path_omits_volume_flag_by_default() {
	// A plain `down` (or scale-down) must not drop volumes.
	let path = container_rm_path("proj-web-1", false);
	assert!(path.ends_with("/proj-web-1?force=true"), "got: {path}");
	assert!(!path.contains("v=true"), "got: {path}");
}

#[test]
fn rm_path_requests_anonymous_volume_removal() {
	// `down -v` must pass `v=true` so podman reclaims the container's
	// anonymous (image VOLUME / short-form) volumes.
	let path = container_rm_path("proj-web-1", true);
	assert!(path.contains("force=true"), "got: {path}");
	assert!(path.contains("&v=true"), "got: {path}");
}

#[test]
fn rm_path_url_encodes_container_name() {
	// Names are URL-encoded so a slash in a container name cannot alter the
	// request path.
	let path = container_rm_path("weird/name", true);
	assert!(!path.contains("weird/name"), "got: {path}");
	assert!(path.contains("weird%2Fname"), "got: {path}");
}
