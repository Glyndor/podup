//! Engine integration tests for `logs` and `attach_logs`.
use super::*;
/// Same shape as `ps`: `Engine::logs` streams to stdout and returns
/// `Result<()>`. The output contract (the container's own line, carrying the
/// `service |` prefix that #594 dropped) is asserted in
/// `cli_logs::cli_logs_subcommand`.
#[tokio::test]
async fn logs_from_named_service() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("lgs");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo hello && sleep infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.logs(&file, Some("web"), false).await.unwrap();
	engine.down(&file).await.unwrap();
}

/// The all-services variant of the above, and unassertable here for the same
/// reason. Kept because it exercises the no-target branch of the same call.
#[tokio::test]
async fn logs_all_services() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("lga");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.logs(&file, None, false).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn logs_unknown_service_fails() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("lgf");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let err = engine
		.logs(&file, Some("nonexistent"), false)
		.await
		.unwrap_err();
	assert!(matches!(err, podup::ComposeError::ServiceNotFound(_)));
}

#[tokio::test]
async fn attach_logs_empty_attach_returns() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("atl");
	let engine = Engine::new(client, proj.clone());
	// attach: false, so attach_logs finds no targets and returns immediately
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    attach: false\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// "Returns immediately" is the claim, and it is the one thing here that can be
	// checked without reading the stream: attaching to a service marked
	// `attach: false` would follow the container's output and never come back, so
	// the test would hang rather than fail. A bound turns that into a red.
	let started = tokio::time::Instant::now();
	let attached = tokio::time::timeout(
		std::time::Duration::from_secs(10),
		engine.attach_logs(&file),
	)
	.await;
	let elapsed = started.elapsed();
	engine.down(&file).await.unwrap();

	let attached =
		attached.expect("attach_logs did not return within 10s on an attach: false service");
	attached.unwrap();
	assert!(
		elapsed < std::time::Duration::from_secs(5),
		"attach_logs took {elapsed:?} on a service with attach: false; it has no targets and must not wait"
	);
}
