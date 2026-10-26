//! Engine integration tests for the container each command reaches when the
//! service is scaled to more than one replica.
use super::*;
// ---------------------------------------------------------------------------
// Replicas: restart, logs, top, exec, port target correct containers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn restart_scaled_service_all_replicas() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("rsr");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo start >> /starts; sleep infinity\"]\n    deploy:\n      replicas: 2\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	let one = format!("{proj}-worker-1");
	let two = format!("{proj}-worker-2");
	// Wait for BOTH replicas to have written their first line before restarting.
	// Waiting on only one of them is a race: the other can still be starting when
	// the restart fires, and then it ends up with one line instead of two and the
	// check below fails for a reason that has nothing to do with restart. It
	// passed alone and failed under the load of the whole file, which is the
	// shape that makes this kind of fixture bug look like flakiness.
	let started_one = poll_container_file(&engine, &one, "/starts", "start", 30).await;
	let started_two = poll_container_file(&engine, &two, "/starts", "start", 30).await;

	// Both replicas must be reachable for restart to succeed. "All replicas" is
	// the claim, and restarting only the first satisfied the old version.
	engine.restart(&file, Some("worker")).await.unwrap();
	let first_restarted = poll_container_file(&engine, &one, "/starts", "start\nstart", 60).await;
	let second_restarted = poll_container_file(&engine, &two, "/starts", "start\nstart", 60).await;

	engine.down(&file).await.unwrap();
	assert!(
		started_one && started_two,
		"a replica never wrote its first start line (one: {started_one}, two: {started_two})"
	);
	assert!(first_restarted, "the first replica did not restart");
	assert!(
		second_restarted,
		"restart stopped at the first replica and left the second running"
	);
}

/// Unassertable here and, unlike `top` and `images`, **not covered anywhere
/// else either**. `logs` on a scaled service prints, so the library returns
/// nothing to check, and no CLI test drives a scaled project through it. The
/// claim in the name, that every replica is included, is currently unverified.
/// Closing that needs a CLI-level test over `replicas: 2`, not an assertion here.
#[tokio::test]
async fn logs_scaled_service_all_replicas() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("lsr");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo hello && sleep infinity\"]\n    deploy:\n      replicas: 2\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// logs for a named service with replicas: should stream from all without error.
	engine.logs(&file, Some("worker"), false).await.unwrap();
	engine.down(&file).await.unwrap();
}

/// Same gap as the scaled `logs` above: `cli_top_subcommand` drives a
/// single-service project, so "all replicas" is asserted nowhere.
#[tokio::test]
async fn top_scaled_service_all_replicas() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("tsr");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    deploy:\n      replicas: 2\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.top(&file, &[]).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn exec_scaled_service_targets_first_replica() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("esr");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    deploy:\n      replicas: 2\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// Leave a mark instead of echoing, and read it from BOTH replicas. "targets
	// the first replica" has two halves, and an exec that hit replica 2, or one
	// that somehow hit both, returned Ok just as happily as the right one.
	engine
		.exec_with_options(
			&file,
			"worker",
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"echo reached > /which".to_string(),
			],
			podup::ExecOptions::default(),
		)
		.await
		.unwrap();
	let first = engine
		.test_exec_capture(
			&format!("{proj}-worker-1"),
			vec!["cat".into(), "/which".into()],
		)
		.await
		.unwrap_or_default();
	let second = engine
		.test_exec_capture(
			&format!("{proj}-worker-2"),
			vec!["cat".into(), "/which".into()],
		)
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();

	assert_eq!(
		first.trim(),
		"reached",
		"exec on a scaled service did not run in the first replica"
	);
	assert_ne!(
		second.trim(),
		"reached",
		"exec on a scaled service ran in the second replica too"
	);
}

#[tokio::test]
async fn port_scaled_service_targets_first_replica() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("psr");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    ports:\n      - \"80\"\n    deploy:\n      replicas: 2\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.port(&file, "worker", "80", "tcp").await.unwrap();
	// --index targets a valid replica; an out-of-range index errors.
	engine
		.port_with_index(&file, "worker", "80", "tcp", Some(2))
		.await
		.unwrap();
	let bad = engine
		.port_with_index(&file, "worker", "80", "tcp", Some(9))
		.await;
	assert!(
		matches!(bad, Err(podup::ComposeError::ServiceNotFound(_))),
		"out-of-range port --index must error, got {bad:?}"
	);
	engine.down(&file).await.unwrap();
}
