use super::engine_with;
use crate::compose::types::{ComposeFile, Service};
use crate::engine::fake_podman;
use crate::error::ComposeError;

/// Two containers to tear down: one whose removal genuinely fails (a busy
/// mount, an active exec session), one that removes cleanly. `down` must
/// still attempt (and complete) the second before exiting non-zero for the
/// first (#598): a CI teardown must not be told it succeeded.
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

/// #598 regression: `container_rm_path` always forces removal (`?force=true`),
/// which SIGKILLs the container regardless of how `stop` went, so a stop
/// that fails or stalls (HTTP 500) is superseded, not fatal, once the
/// force-remove that follows it succeeds. This pins the exact gap that let the
/// bug through: folding the `stop` failure into `first_err` made `down` return
/// `Err` even though teardown fully succeeded.
#[tokio::test]
#[cfg(unix)]
async fn down_tolerates_a_stalled_stop_when_the_force_remove_succeeds() {
	let containers = r#"[{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}}]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else if method == "POST" && target.contains("/stop") {
			(
				500,
				r#"{"message":"timed out waiting for container to exit"}"#.to_string(),
			)
		} else if method == "DELETE" && target.contains("/proj-web-1?force=true") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	e.down_with_options(&file, false)
		.await
		.expect("a stalled/failed stop superseded by a successful force-remove must not fail down");

	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.any(|r| r.contains("POST") && r.contains("/stop")),
		"expected the stop to have been attempted: {seen:?}"
	);
	assert!(
		seen.iter()
			.any(|r| r.contains("DELETE") && r.contains("/proj-web-1?force=true")),
		"expected the force-remove to have been attempted: {seen:?}"
	);
}

/// A second `down` on an already torn-down project (no live containers,
/// nothing left to sweep) must still exit 0; idempotency is preserved.
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

/// `down --rmi` used to warn and return Ok on a real removal failure, so it
/// reported success having left images behind, the one arm of this teardown
/// that did not aggregate, while its network, volume and container siblings all
/// did.
#[cfg(unix)]
#[tokio::test]
async fn remove_service_images_reports_a_real_failure_after_trying_them_all() {
	let attempted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
	let seen = std::sync::Arc::clone(&attempted);
	let fake = fake_podman::start(move |method, target| {
		if method == "DELETE" && target.contains("/images/") {
			seen.lock().expect("lock").push(target.to_string());
			// A 500 that is neither 404 nor an in-use conflict is a real failure.
			if target.contains("broken") {
				return (500, r#"{"message":"internal error"}"#.to_string());
			}
			return (200, String::new());
		}
		(404, r#"{"message":"not found"}"#.to_string())
	});
	let engine = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert(
		"a".into(),
		Service {
			image: Some("broken:latest".into()),
			..Default::default()
		},
	);
	file.services.insert(
		"b".into(),
		Service {
			image: Some("fine:latest".into()),
			..Default::default()
		},
	);

	let err = engine
		.remove_service_images(&file, false)
		.await
		.expect_err("a real image-removal failure must propagate");
	assert!(
		matches!(err, ComposeError::Podman(ref pe) if pe.is_status(500)),
		"got: {err}"
	);
	assert_eq!(
		attempted.lock().expect("lock").len(),
		2,
		"the sweep must finish before reporting: both images are attempted"
	);
}

/// A network the project does not own is refused, but the volumes below it are
/// still the project's to remove. The refusal used to return at once, so
/// `down -v` stopped before reaching them and left the data it was asked to
/// delete.
#[tokio::test]
#[cfg(unix)]
async fn a_refused_network_does_not_stop_the_volumes_being_removed() {
	let fake = fake_podman::start(move |method, target| {
		if method == "GET"
			&& (target.contains("/containers/json") || target.contains("/networks/json"))
		{
			(200, "[]".to_string())
		} else if method == "GET" && target.contains("/networks/") {
			// Exists, carries no ownership label.
			(200, r#"{"name":"proj_net"}"#.to_string())
		} else if method == "GET" && target.contains("/volumes/") {
			(200, r#"{"Labels":{"podup.project":"proj"}}"#.to_string())
		} else if method == "DELETE" && target.contains("/volumes/proj_vol") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let file =
		crate::parse_str("services:\n  web:\n    image: x\nnetworks:\n  net:\nvolumes:\n  vol:\n")
			.unwrap();

	let err = e
		.down_with_options(&file, true)
		.await
		.expect_err("the refusal is still reported");
	assert!(err.to_string().contains("proj_net"), "{err}");

	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.any(|r| r.contains("DELETE") && r.contains("/volumes/proj_vol")),
		"the volume was not removed after the network was refused: {seen:?}"
	);
	assert!(
		!seen
			.iter()
			.any(|r| r.contains("DELETE") && r.contains("/networks/proj_net")),
		"a network the project does not own was removed: {seen:?}"
	);
}
