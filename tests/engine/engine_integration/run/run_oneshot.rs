//! Engine integration tests for `run` on a service that is not running.
use super::*;
// ---------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engine_run_command_succeeds() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("run");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str("services:\n  job:\n    image: alpine:latest\n").unwrap();

	let result = engine
		.run(
			&file,
			"job",
			podup::RunOptions::new(
				vec!["echo".to_string(), "hello".to_string()],
				true,
				false,
				Vec::new(),
				None,
				false,
			),
		)
		.await;
	assert!(result.is_ok(), "run failed");
}

#[tokio::test]
async fn engine_run_nonzero_exit_returns_run_exited() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("rxc");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str("services:\n  job:\n    image: alpine:latest\n").unwrap();

	let result = engine
		.run(
			&file,
			"job",
			podup::RunOptions::new(
				vec!["false".to_string()],
				true,
				false,
				Vec::new(),
				None,
				false,
			),
		)
		.await;
	// Split rather than flattened. Dropping the formatted value would otherwise
	// lose the difference between "succeeded" and "failed with the wrong
	// variant"; `expect_err` prints the `Ok` value, which here is `()`.
	let err = result.expect_err("expected RunExited, got Ok");
	assert!(
		matches!(err, podup::ComposeError::RunExited(_)),
		"expected RunExited, got a different ComposeError variant"
	);
}
