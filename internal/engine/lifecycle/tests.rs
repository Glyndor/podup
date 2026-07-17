use super::container_rm_path;

#[cfg(unix)]
use super::Engine;
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
