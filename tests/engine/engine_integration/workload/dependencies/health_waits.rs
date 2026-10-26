//! Engine integration tests for the health and completion waits `up` performs on a
//! dependency.

use super::*;
// Health: non-zero exit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wait_completed_nonzero_error() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("cne");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  init:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"exit 1\"]\n  app:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      init:\n        condition: service_completed_successfully\n",
	)
	.unwrap();

	// up() propagates the non-zero exit error from wait_completed
	let err = engine.up(&file).await.unwrap_err();
	assert!(
		matches!(err, podup::ComposeError::HealthCheckTimeout(_)),
		"expected HealthCheckTimeout, got: {err}"
	);
	let _ = engine.down(&file).await;
}

// ---------------------------------------------------------------------------
// Health: wait_healthy timeout and wait_completed polling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wait_healthy_times_out() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("wht");
	let engine = Engine::new(client, proj.clone());
	// CMD false always fails; retries:1 means wait_healthy exhausts quickly
	// Covers health.rs L42-43 (loop body closing braces) and L47 (timeout Err)
	let file = parse_str(
		"services:\n  db:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    healthcheck:\n      test: [\"CMD\", \"false\"]\n      interval: 1s\n      timeout: 1s\n      retries: 1\n      start_period: 0s\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      db:\n        condition: service_healthy\n",
	)
	.unwrap();

	let err = engine.up(&file).await.unwrap_err();
	assert!(
		matches!(err, podup::ComposeError::HealthCheckTimeout(_)),
		"expected HealthCheckTimeout, got: {err}"
	);
	let _ = engine.down(&file).await;
}

#[tokio::test]
async fn wait_completed_polling() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("wcp");
	let engine = Engine::new(client, proj.clone());
	// init takes a while before exiting, so the first poll sees it running and the
	// polling loop is exercised rather than short-circuited. The delay has to beat
	// the cost of creating app's container, or `up` would appear ordered even with
	// the wait removed, the reason 2s was not enough in
	// depends_on_health::depends_on_service_completed.
	// The bind uses an absolute path, so no base_dir is needed.
	let dir = tempfile::tempdir().unwrap();
	let yaml = format!(
		"services:\n  init:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"sleep 12; echo init-done > /out/order; exit 0\"]\n    volumes:\n      - {out}:/out:z\n  app:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"cat /out/order > /out/app-saw 2>/dev/null; sleep infinity\"]\n    volumes:\n      - {out}:/out:z\n    depends_on:\n      init:\n        condition: service_completed_successfully\n",
		out = dir.path().display()
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	let saw = poll_host_file(dir.path().join("app-saw"), "init-done", 30).await;
	engine.down(&file).await.unwrap();

	assert!(
		saw,
		"app started before init completed, so the polling wait was not honoured"
	);
}
