//! Tear-down half of the lifecycle suite: `down` across levels, the
//! `container_rm_path` query-string shape, and image removal.
//!
//! Split out of `up/tests.rs` for the reason `scale/scale_tests.rs` and
//! `lifecycle/drop_recheck_tests.rs` were: that file had reached 487 code lines against a
//! 500-line hard limit, so the pull request that broke the build would have
//! been whichever one next added a lifecycle test, not the one that made the
//! file long.
//!
//! The cut is by subject, not by size. What stays in `up/tests.rs` brings a
//! project up; what moves here takes one down.
//!
//! Same fixture pattern as the rest of the lifecycle suite, including the local
//! `engine_with`, which `scale/scale_tests.rs` also carries rather than sharing: three
//! lines duplicated read better here than a helper exported out of a test module.

use crate::engine::lifecycle::container_rm_path;

#[cfg(unix)]
use crate::compose::types::{ComposeFile, Service};
#[cfg(unix)]
use crate::engine::fake_podman;
#[cfg(unix)]
use crate::engine::lifecycle::Engine;
#[cfg(unix)]
use crate::error::ComposeError;

// Guarded like everything it supports: the `down` fixtures need a unix-socket
// fake libpod, while the `container_rm_path` tests below are pure string
// assertions and run everywhere. Without the guard this helper is dead code on
// Windows, where the whole suite is built with `-D warnings`.
#[cfg(unix)]
fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

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

/// `down` now walks dependency levels (reversed) instead of a flat reversed
/// order, fanning out within a level via `join_bounded`. `web depends_on db`
/// puts web alone in the first (post-reversal) level and db alone in the
/// second, so this isolates the cross-level ordering guarantee from
/// within-level concurrency: web's whole teardown (stop + rm) must complete
/// before db is even asked to stop, exactly like the pre-parallel flat
/// reversed-order walk.
#[tokio::test]
#[cfg(unix)]
async fn down_tears_down_dependent_levels_before_their_dependencies() {
	let containers = r#"[
		{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-db-1"],"Labels":{"podup.service":"db"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else if (method == "POST" && target.contains("/stop"))
			|| (method == "DELETE" && target.contains("force=true"))
		{
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let file = crate::parse_str(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    depends_on:\n      - db\n",
	)
	.unwrap();

	e.down_with_options(&file, false)
		.await
		.expect("a healthy two-level teardown must succeed");

	let seen = fake.requests.lock().unwrap();
	let web_rm = seen
		.iter()
		.position(|r| r.contains("DELETE") && r.contains("proj-web-1?force=true"))
		.expect("web must have been removed");
	let db_stop = seen
		.iter()
		.position(|r| r.contains("POST") && r.contains("proj-db-1") && r.contains("stop"))
		.expect("db must have been stopped");
	assert!(
		web_rm < db_stop,
		"expected web's level to fully complete before db's level's stop begins: {seen:?}"
	);
}

/// `web` and `cache` share no `depends_on` relationship, so `resolve_levels`
/// groups them into a single level and `down_with_options` dispatches both
/// containers' teardown through the same `join_bounded` (`buffer_unordered`)
/// mechanism proven order-independent by
/// `parallel::tests::dispatch::join_bounded_preserves_input_order`, instead of one
/// await-per-container in strict sequence. Asserting real wall-clock overlap
/// against a synchronous test responder would require a multi-thread runtime
/// and a blocking rendezvous inside the fake, a source of exactly the
/// flakiness the testing standard forbids, so this test instead pins the
/// dispatch contract (both containers are targeted, the level completes) and
/// leaves the concurrency guarantee itself to `join_bounded`'s own test plus
/// the code structure (no `.await` between the two containers' futures).
#[tokio::test]
#[cfg(unix)]
async fn down_targets_every_independent_service_within_one_level() {
	let containers = r#"[
		{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-cache-1"],"Labels":{"podup.service":"cache"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else if (method == "POST" && target.contains("/stop"))
			|| (method == "DELETE" && target.contains("force=true"))
		{
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());
	file.services.insert("cache".into(), Service::default());

	e.down_with_options(&file, false)
		.await
		.expect("a healthy single-level teardown of two independent services must succeed");

	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.any(|r| r.contains("DELETE") && r.contains("proj-web-1?force=true")),
		"expected web to have been targeted: {seen:?}"
	);
	assert!(
		seen.iter()
			.any(|r| r.contains("DELETE") && r.contains("proj-cache-1?force=true")),
		"expected cache to have been targeted: {seen:?}"
	);
}

/// Determinism regression: with `web depends_on db`, both containers' removal
/// fail with distinct statuses. Levels are visited in a fixed order (web's
/// level first, post-reversal), so `down` must return web's failure, never
/// db's, regardless of how `join_bounded`'s internal `buffer_unordered`
/// happens to interleave completions within each level. db's level must still
/// be attempted (best-effort teardown continues past the first failing
/// level).
#[tokio::test]
#[cfg(unix)]
async fn down_first_error_is_deterministic_across_levels() {
	let containers = r#"[
		{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-db-1"],"Labels":{"podup.service":"db"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else if method == "POST" && target.contains("/stop") {
			(200, String::new())
		} else if method == "DELETE" && target.contains("proj-web-1?force=true") {
			(500, r#"{"message":"web busy"}"#.to_string())
		} else if method == "DELETE" && target.contains("proj-db-1?force=true") {
			(503, r#"{"message":"db unavailable"}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let file = crate::parse_str(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    depends_on:\n      - db\n",
	)
	.unwrap();

	let err = e
		.down_with_options(&file, false)
		.await
		.expect_err("both levels fail to remove their container");
	assert!(
		matches!(err, ComposeError::Podman(ref pe) if pe.is_status(500)),
		"expected web's (first, post-reversal) level failure, not db's: {err:?}"
	);

	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.any(|r| r.contains("DELETE") && r.contains("proj-db-1?force=true")),
		"expected db's level to still be attempted after web's level failed: {seen:?}"
	);
}

#[test]
fn rm_path_omits_volume_flag_by_default() {
	// A plain `down` (or scale-down) must not drop volumes.
	let path = container_rm_path("proj-web-1", false, true);
	assert!(path.ends_with("/proj-web-1?force=true"), "got: {path}");
	assert!(!path.contains("volumes="), "got: {path}");
}

#[test]
fn rm_path_requests_anonymous_volume_removal() {
	// `down -v` must pass `volumes=true` so podman reclaims the container's
	// anonymous (image VOLUME / short-form) volumes. The libpod
	// `/containers/{id}` delete endpoint reads `volumes=` for that; the
	// Docker compat handler used `v=`, which the libpod handler ignores
	// (so a `down -v` against libpod without `volumes=` reclaims nothing).
	let path = container_rm_path("proj-web-1", true, true);
	assert!(path.contains("force=true"), "got: {path}");
	assert!(path.contains("&volumes=true"), "got: {path}");
}

#[test]
fn rm_path_url_encodes_container_name() {
	// Names are URL-encoded so a slash in a container name cannot alter the
	// request path.
	let path = container_rm_path("weird/name", true, true);
	assert!(!path.contains("weird/name"), "got: {path}");
	assert!(path.contains("weird%2Fname"), "got: {path}");
}

#[test]
fn rm_path_force_false_passes_force_false() {
	// `podup rm` (no `-f`) must not force a running container down; the libpod
	// delete endpoint honours `force=false` by rejecting a still-running
	// container with a 409. The shared helper passes the caller's flag
	// through.
	let path = container_rm_path("proj-web-1", false, false);
	assert!(path.contains("force=false"), "got: {path}");
	assert!(!path.contains("force=true"), "got: {path}");
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

/// The stop at the end of a foreground `up` is for the containers it was
/// attached to. A container that took the name since, put there by another
/// `up`, is a different container and is left running.
#[tokio::test]
#[cfg(unix)]
async fn stop_snapshot_leaves_a_container_that_replaced_one_of_the_snapshot() {
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::sync::Arc;
	let listings = Arc::new(AtomicUsize::new(0));
	let seen = listings.clone();
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			// The first listing is the snapshot; later ones show web-1 replaced.
			let id = if seen.fetch_add(1, Ordering::SeqCst) == 0 {
				"aaaa"
			} else {
				"bbbb"
			};
			(
				200,
				format!(
					r#"[
						{{"Id":"{id}","Names":["/proj-web-1"],"Labels":{{"podup.service":"web"}}}},
						{{"Id":"cccc","Names":["/proj-db-1"],"Labels":{{"podup.service":"db"}}}}
					]"#
				),
			)
		} else if method == "POST" && target.contains("/stop") {
			(204, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());
	file.services.insert("db".into(), Service::default());

	let snapshot = e.snapshot_containers().await.unwrap();
	e.stop_snapshot(&file, &snapshot).await.unwrap();

	let seen = fake.requests.lock().unwrap();
	let posts: Vec<_> = seen.iter().filter(|r| r.starts_with("POST ")).collect();
	assert_eq!(posts.len(), 1, "{seen:?}");
	let expected = format!("POST {}/containers/cccc/stop", crate::libpod::API_PREFIX);
	assert_eq!(posts[0].split('?').next(), Some(expected.as_str()));
}

#[tokio::test]
#[cfg(unix)]
async fn stop_snapshot_posts_to_the_id_even_when_the_name_changes_owner_after_the_listing() {
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::sync::Arc;

	for replaced_at_post in [false, true] {
		let listings = Arc::new(AtomicUsize::new(0));
		let listed = listings.clone();
		let fake = fake_podman::start(move |method, target| {
			if method == "GET" && target.contains("/containers/json") {
				listed.fetch_add(1, Ordering::SeqCst);
				return (
					200,
					r#"[{"Id":"aaaa","Names":["/proj-web-1"],"Labels":{"podup.service":"web"}}]"#
						.into(),
				);
			}
			if method == "POST" && target.contains("/stop") {
				// At POST time the old ID may be absent while the name has a new owner.
				if replaced_at_post && target.contains("/containers/aaaa/stop?") {
					return (404, r#"{"message":"not found"}"#.into());
				}
				return (204, String::new());
			}
			(404, r#"{"message":"not found"}"#.into())
		});
		let e = engine_with(fake.client(), "proj");
		let mut file = ComposeFile::default();
		file.services.insert("web".into(), Service::default());

		let snapshot = e.snapshot_containers().await.unwrap();
		e.stop_snapshot(&file, &snapshot).await.unwrap();

		assert_eq!(listings.load(Ordering::SeqCst), 2);
		let seen = fake.requests.lock().unwrap();
		let posts: Vec<_> = seen.iter().filter(|r| r.starts_with("POST ")).collect();
		assert_eq!(posts.len(), 1, "{seen:?}");
		let expected = format!("POST {}/containers/aaaa/stop", crate::libpod::API_PREFIX);
		assert_eq!(posts[0].split('?').next(), Some(expected.as_str()));
	}
}

#[tokio::test]
#[cfg(unix)]
async fn stop_snapshot_excludes_one_off_containers_of_the_same_service() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, r#"[
				{"Id":"aaaa","Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
				{"Id":"bbbb","Names":["/proj-web-run-123"],"Labels":{"podup.service":"web","podup.oneoff":"true"}}
			]"#.into())
		} else if method == "POST" && target.contains("/stop") {
			(204, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.into())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	let snapshot = e.snapshot_containers().await.unwrap();
	e.stop_snapshot(&file, &snapshot).await.unwrap();

	let seen = fake.requests.lock().unwrap();
	let posts: Vec<_> = seen.iter().filter(|r| r.starts_with("POST ")).collect();
	assert_eq!(posts.len(), 1, "{seen:?}");
	let expected = format!("POST {}/containers/aaaa/stop", crate::libpod::API_PREFIX);
	assert_eq!(posts[0].split('?').next(), Some(expected.as_str()));
}
