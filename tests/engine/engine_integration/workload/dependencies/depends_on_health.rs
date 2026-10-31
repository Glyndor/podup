//! Engine integration tests for the `depends_on` readiness conditions: waiting
//! on a healthcheck, waiting on a completed service, and skipping the wait when
//! the dependency declares no healthcheck.

use super::*;
// ---------------------------------------------------------------------------
// Health / depends_on
// ---------------------------------------------------------------------------

#[tokio::test]
async fn depends_on_service_healthy() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("hlt");
	let dir = tempfile::tempdir().unwrap();
	let engine = Engine::new(client, proj.clone());
	// `CMD true` is healthy from the first probe, so it cannot show whether web
	// waited. db's healthcheck now depends on work db does after a delay, and
	// web's first act is to read what db left, so web starting early finds
	// nothing.
	//
	// Two constraints pull against each other here. The delay has to exceed the
	// time it takes to create web's container, or the mutation that empties the
	// readiness map still leaves this green (see depends_on_service_completed).
	// And `retries` has to outlast that same delay, or db is declared unhealthy
	// before it writes and `up` fails before any assertion runs, which is what
	// 12s against 10 retries at a 1s interval did.
	let yaml = format!(
		"services:\n  db:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"sleep 12; echo db-ready > /out/ready; sleep infinity\"]\n    volumes:\n      - {out}:/out:z\n    healthcheck:\n      test: [\"CMD\", \"test\", \"-f\", \"/out/ready\"]\n      interval: 1s\n      timeout: 2s\n      retries: 30\n      start_period: 0s\n  web:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"cat /out/ready > /out/web-saw 2>/dev/null; sleep infinity\"]\n    volumes:\n      - {out}:/out:z\n    depends_on:\n      db:\n        condition: service_healthy\n",
		out = dir.path().display()
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	let saw = poll_host_file(dir.path().join("web-saw"), "db-ready", 30).await;
	engine.down(&file).await.unwrap();

	assert!(
		saw,
		"web started before db reported healthy, so service_healthy was not waited on"
	);
}

#[tokio::test]
async fn depends_on_service_healthy_with_default_timeout() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("hltdef");
	let engine = Engine::new(client, proj.clone());
	// db's healthcheck OMITS `timeout`/`retries`. Podman defaults a missing
	// Timeout to 0s (every probe fails "exceeded timeout of 0s"), so without the
	// compose-spec default the db never becomes healthy and this up would hang.
	let file = parse_str(
		"services:\n  db:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    healthcheck:\n      test: [\"CMD\", \"true\"]\n      interval: 1s\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      db:\n        condition: service_healthy\n",
	)
	.unwrap();

	// The comment above states the failure mode exactly: without the compose-spec
	// default, Podman treats a missing Timeout as 0s, every probe fails "exceeded
	// timeout of 0s", db never reports healthy and this `up` hangs until the
	// health deadline. A hang is not something a bare `.unwrap()` can report, so
	// bound it.
	let started = std::time::Instant::now();
	engine.up(&file).await.unwrap();
	let elapsed = started.elapsed();
	let mut names = engine
		.test_project_container_names()
		.await
		.unwrap_or_default();
	names.sort();
	engine.down(&file).await.unwrap();

	assert!(
		elapsed < std::time::Duration::from_secs(45),
		"up took {elapsed:?}; a healthcheck with no explicit timeout must get the spec default, not 0s"
	);
	assert_eq!(
		names,
		vec![format!("{proj}-db-1"), format!("{proj}-web-1")],
		"web did not come up behind a healthcheck that omits timeout and retries"
	);
}

#[tokio::test]
async fn depends_on_service_completed() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("cmp");
	let dir = tempfile::tempdir().unwrap();
	let engine = Engine::new(client, proj.clone());
	// Ordering is the contract, and `up` returning Ok says nothing about it. init
	// takes a while and then leaves a file; app's first act is to read that file.
	//
	// The delay has to beat the cost of creating app's container, not just be
	// non-zero. `up` starts services in dependency levels, so init is launched
	// before app whether or not the readiness wait happens; the wait only adds
	// "and has finished". With a 2s delay the mutation that empties the readiness
	// map left this test green, because creating app took longer than that on its
	// own.
	// If app were started before init completed, the read finds nothing and the
	// marker below stays empty, which is exactly what a dropped dependency wait
	// looks like from outside.
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
		"app started before init had completed, so service_completed_successfully was not waited on"
	);
}

// Regression: a dependency scaled to >1 has no base-named container, only
// `{base}-1..N`. The readiness wait must target the first replica, not the
// (nonexistent) base name, or `up` 404s and aborts despite the dep completing.
#[tokio::test]
async fn depends_on_scaled_service_completed() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("cmpscale");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  init:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"exit 0\"]\n    deploy:\n      replicas: 2\n  app:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      init:\n        condition: service_completed_successfully\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let mut names = engine
		.test_project_container_names()
		.await
		.unwrap_or_default();
	names.sort();
	engine.down(&file).await.unwrap();

	// The comment above names the regression precisely: a scaled dependency has no
	// container under the base name, so a readiness wait aimed at `init` rather
	// than `init-1` 404s and aborts `up`.
	//
	// Measured: aiming the wait at the base name reddens this test at
	// `up().unwrap()`, not here, so the bare unwrap already covered that
	// regression. What the assertion adds is that both replicas exist and app
	// came up with them, which the unwrap could not distinguish from a run where
	// app was silently skipped. Precision, not new falsifiability.
	assert_eq!(
		names,
		vec![
			format!("{proj}-app-1"),
			format!("{proj}-init-1"),
			format!("{proj}-init-2"),
		],
		"a scaled completed-successfully dependency did not resolve to both replicas plus the dependant"
	);
}

// ---------------------------------------------------------------------------
// Depends-on: service_healthy with no healthcheck
// ---------------------------------------------------------------------------

#[tokio::test]
async fn depends_on_healthy_no_healthcheck_skips_wait() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("hns");
	let engine = Engine::new(client, proj.clone());
	// backend has no healthcheck; frontend depends on it with service_healthy.
	// podup logs a debug message and skips the wait.
	let file = parse_str(
		"services:\n  backend:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  frontend:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      backend:\n        condition: service_healthy\n",
	)
	.unwrap();

	// "Skips the wait" is the claim, and the way it fails is by NOT skipping:
	// waiting on a container that has no healthcheck can only end in the timeout,
	// so the test would take the full health deadline and then error rather than
	// fail. A bound turns that into an assertion.
	let started = std::time::Instant::now();
	engine.up(&file).await.unwrap();
	let elapsed = started.elapsed();
	let mut names = engine
		.test_project_container_names()
		.await
		.unwrap_or_default();
	names.sort();
	engine.down(&file).await.unwrap();

	assert!(
		elapsed < std::time::Duration::from_secs(30),
		"up took {elapsed:?}; a service_healthy dependency with no healthcheck must be skipped, not waited on"
	);
	assert_eq!(
		names,
		vec![format!("{proj}-backend-1"), format!("{proj}-frontend-1")],
		"the dependant did not come up despite the healthcheck-less dependency being skipped"
	);
}
