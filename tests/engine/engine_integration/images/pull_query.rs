//! Engine integration tests for `pull`, with and without `--ignore-pull-failures`.
use super::*;
/// `Engine::pull` reports progress on stderr and returns `Result<()>`. Asserting
/// that the image is present afterwards would prove nothing: `alpine:latest` is
/// already local in every environment this suite runs in, so the assertion would
/// hold with the pull removed entirely. Removing it first would make the test
/// depend on the network, which the testing standard rules out. What `pull`
/// reports is asserted in `cli_pull::cli_pull_subcommand`.
#[tokio::test]
async fn pull_images() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("pll");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.pull(&file).await.unwrap();
}

#[tokio::test]
async fn pull_ignore_failures_continues_past_bad_image() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("plif");
	let engine = Engine::new(client, proj.clone());
	// A bogus registry/image alongside a good one: the bad pull fails.
	let file = parse_str(
		"services:\n  good:\n    image: alpine:latest\n  bad:\n    image: localhost:1/nope:nope\n",
	)
	.unwrap();

	// Without --ignore-pull-failures the bad image aborts the whole pull.
	let strict = engine.pull(&file).await;
	assert!(strict.is_err(), "bad image must fail a strict pull");

	// With --ignore-pull-failures the failure is logged and pull returns Ok.
	let lenient = engine
		.pull_services_with_options(&file, &[], podup::PullOptions::new(true, false))
		.await;
	assert!(
		lenient.is_ok(),
		"ignore-pull-failures must not abort: {lenient:?}"
	);
}
