use super::engine_with;
use crate::compose::types::{ComposeFile, Service};
use crate::engine::fake_podman;
use crate::error::ComposeError;

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
