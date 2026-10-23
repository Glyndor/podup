#[cfg(unix)]
use crate::engine::fake_podman;

/// #8: `pull_services_with_options` used to build one future per service,
/// so two services sharing an image pulled it twice. They must now
/// dedupe down to a single pull request, with both services still
/// reported as successful.
#[tokio::test]
#[cfg(unix)]
async fn pull_dedupes_a_shared_image_into_a_single_pull() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.contains("/images/pull") {
			(200, String::new())
		} else if method == "GET" && target.contains("/images/") && target.contains("/json") {
			(200, "{}".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = crate::engine::Engine::new(fake.client(), "proj".into());
	let file =
		crate::parse_str("services:\n  a:\n    image: shared\n  b:\n    image: shared\n").unwrap();

	e.pull_services(&file, &[])
		.await
		.expect("pulling two services that share an image must succeed");

	let seen = fake.requests.lock().unwrap();
	let pulls = seen
		.iter()
		.filter(|r| r.contains("/images/pull") && r.contains("reference=shared"))
		.count();
	assert_eq!(
		pulls, 1,
		"two services sharing one image must issue a single pull: {seen:?}"
	);
}

/// Two services sharing an image but with *different* resolved pull
/// policies (no `--pull` override) must each get their own pull request,
/// not collapse into one: the dedup key must include the resolved
/// policy, not just the image reference.
#[tokio::test]
#[cfg(unix)]
async fn pull_issues_separate_requests_for_same_image_different_policy() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.contains("/images/pull") {
			(200, String::new())
		} else if method == "GET" && target.contains("/images/") && target.contains("/json") {
			(200, "{}".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = crate::engine::Engine::new(fake.client(), "proj".into());
	let file = crate::parse_str(
		"services:\n  a:\n    image: shared\n    pull_policy: never\n  b:\n    image: shared\n    pull_policy: always\n",
	)
	.unwrap();

	e.pull_services(&file, &[])
		.await
		.expect("differing per-service pull_policy must not fail the pull");

	let seen = fake.requests.lock().unwrap();
	let pulls: Vec<&String> = seen
		.iter()
		.filter(|r| r.contains("/images/pull") && r.contains("reference=shared"))
		.collect();
	assert_eq!(
		pulls.len(),
		2,
		"same image with different resolved policies must issue two pulls, not one: {seen:?}"
	);
	assert!(
		pulls.iter().any(|r| r.contains("policy=never")),
		"missing the never-policy pull: {seen:?}"
	);
	assert!(
		pulls.iter().any(|r| r.contains("policy=always")),
		"missing the always-policy pull: {seen:?}"
	);
}

/// A shared image that fails to pull must still be reported for *every*
/// service that names it, derived from the one shared outcome, not from
/// a redundant pull per service. `ignore_failures` lets both warnings
/// through instead of aborting on the first.
#[tokio::test]
#[cfg(unix)]
async fn pull_failure_on_a_shared_image_is_still_only_pulled_once() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.contains("/images/pull") {
			(500, r#"{"message":"registry unreachable"}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = crate::engine::Engine::new(fake.client(), "proj".into());
	let file =
		crate::parse_str("services:\n  a:\n    image: shared\n  b:\n    image: shared\n").unwrap();

	let opts = crate::engine::PullOptions {
		ignore_failures: true,
		include_deps: false,
	};
	e.pull_services_with_options(&file, &[], opts)
		.await
		.expect("ignore_failures must not error even though the shared pull failed");

	let seen = fake.requests.lock().unwrap();
	let pulls = seen
		.iter()
		.filter(|r| r.contains("/images/pull") && r.contains("reference=shared"))
		.count();
	assert_eq!(
		pulls, 1,
		"a failing shared image must still be pulled once, not once per service: {seen:?}"
	);
}

/// Without `ignore_failures`, a shared image that never lands must still
/// abort the whole pull: the per-service error report is derived from
/// the image's single shared outcome, so the failure is not silently
/// dropped for services 2..N once service 1 already reported it.
#[tokio::test]
#[cfg(unix)]
async fn pull_failure_on_a_shared_image_aborts_without_ignore_failures() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.contains("/images/pull") {
			(500, r#"{"message":"registry unreachable"}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = crate::engine::Engine::new(fake.client(), "proj".into());
	let file =
		crate::parse_str("services:\n  a:\n    image: shared\n  b:\n    image: shared\n").unwrap();

	let err = e
		.pull_services(&file, &[])
		.await
		.expect_err("a shared image that fails to pull must abort the pull");
	assert!(
		matches!(err, crate::error::ComposeError::Build(ref msg) if msg.contains("shared")),
		"unexpected error: {err:?}"
	);
}

// Bounding the standalone pull's concurrency (`MAX_PULL_CONCURRENCY`) is
// exercised structurally rather than by asserting a live in-flight count:
// `bounded_join_all` runs every future through the same
// `buffer_unordered(MAX_PULL_CONCURRENCY)` dispatcher the lifecycle
// fan-out's `join_bounded` uses (see
// `parallel::tests::dispatch::join_bounded_preserves_input_order`), and a
// synchronous fake responder
// cannot observe real concurrency without a multi-thread runtime and a
// blocking rendezvous, exactly the flakiness the testing standard rules
// out. The dedup tests above already pin the dispatch contract (every
// unique image is attempted, exactly once).
