//! Engine integration tests (split for the source line limit).
use super::*;

// ---------------------------------------------------------------------------
// Pause / unpause
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pause_and_unpause() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("pau");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.pause(&file, &[]).await.unwrap();
	engine.unpause(&file, &[]).await.unwrap();
	engine.down(&file).await.unwrap();
}

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
			podup::RunOptions {
				cmd: vec!["echo".to_string(), "hello".to_string()],
				rm: true,
				detach: false,
				env_overrides: vec![],
				name_override: None,
			},
		)
		.await;
	assert!(result.is_ok(), "run failed: {result:?}");
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
			podup::RunOptions {
				cmd: vec!["false".to_string()],
				rm: true,
				detach: false,
				env_overrides: vec![],
				name_override: None,
			},
		)
		.await;
	assert!(
		matches!(result, Err(podup::ComposeError::RunExited(_))),
		"expected RunExited, got {result:?}"
	);
}

// ---------------------------------------------------------------------------
// Top
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engine_top_running_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("top");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.top(&file, &[]).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engine_images_lists_service_images() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("img");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.images(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Port
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engine_port_returns_binding() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("prt");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    ports:\n      - \"127.0.0.1:18080:80\"\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.port(&file, "web", 80, "tcp").await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Cp
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engine_cp_from_container_extracts_file() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("cpf");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	let dst = dir.path().to_str().unwrap().to_string();
	let src = "web:/etc/hostname".to_string();
	let result = engine.cp(&file, &src, &dst).await;
	engine.down(&file).await.unwrap();

	result.unwrap();
	assert!(dir.path().join("hostname").exists());
}

#[tokio::test]
async fn engine_cp_to_container_uploads_file() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let local_file = dir.path().join("testfile.txt");
	fs::write(&local_file, b"hello from host").unwrap();

	let proj = proj("cpt");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	let src = local_file.to_str().unwrap().to_string();
	let dst = "web:/tmp".to_string();
	let result = engine.cp(&file, &src, &dst).await;
	engine.down(&file).await.unwrap();

	result.unwrap();
}

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
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    deploy:\n      replicas: 2\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// Both replicas must be reachable for restart to succeed.
	engine.restart(&file, Some("worker")).await.unwrap();
	engine.down(&file).await.unwrap();
}

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
	engine
		.exec(&file, "worker", vec!["echo".to_string(), "ok".to_string()])
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
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
	engine.port(&file, "worker", 80, "tcp").await.unwrap();
	engine.down(&file).await.unwrap();
}
