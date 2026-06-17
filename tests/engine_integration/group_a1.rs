//! Engine integration tests (split for the source line limit).
use super::*;

// Lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn up_and_down() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("udn");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn up_no_recreate_skips_running() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("nor");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// Second up with no_recreate: already running → skip
	engine
		.up_with_options(&file, false, &[], &[], true, false, false)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn up_target_services_only() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("tgt");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  db:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      - db\n",
	)
	.unwrap();

	// Only start web (and its dep db)
	engine
		.up_with_options(&file, false, &[], &["web".to_string()], false, false, false)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn down_with_remove_volumes() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("dvol");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(&format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    volumes:\n      - {proj}-data:/data\nvolumes:\n  {proj}-data:\n"
	))
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.down_with_options(&file, true).await.unwrap();
}

#[tokio::test]
async fn restart_all_services() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("rsa");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.restart(&file, None).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn restart_specific_service() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("rss");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.restart(&file, Some("web")).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn restart_cascade_dep() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("rcd");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  db:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      db:\n        condition: service_started\n        restart: true\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// Restarting db triggers cascade restart of web
	engine.restart(&file, Some("db")).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn restart_unknown_service_fails() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("ruf");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let err = engine
		.restart(&file, Some("nonexistent"))
		.await
		.unwrap_err();
	assert!(matches!(err, podup::ComposeError::ServiceNotFound(_)));
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ps_shows_running_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("ps");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.ps(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

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
async fn exec_command_in_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("exc");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine
		.exec_with_options(
			&file,
			"web",
			vec!["echo".to_string(), "test".to_string()],
			podup::ExecOptions::default(),
		)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn exec_with_options_user_workdir_env() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("excopt");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// Podman accepts user/workdir/env on the exec; a bad workdir or user would
	// make this error, so success exercises the option plumbing end to end.
	engine
		.exec_with_options(
			&file,
			"web",
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"pwd; echo $FOO".to_string(),
			],
			podup::ExecOptions {
				user: Some("root".to_string()),
				workdir: Some("/tmp".to_string()),
				env: vec!["FOO=bar".to_string()],
				..Default::default()
			},
		)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn exec_unknown_service_fails() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("exf");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let err = engine
		.exec_with_options(
			&file,
			"nonexistent",
			vec!["echo".to_string()],
			podup::ExecOptions::default(),
		)
		.await
		.unwrap_err();
	assert!(matches!(err, podup::ComposeError::ServiceNotFound(_)));
}

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
async fn remove_orphans_no_orphans() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("orp");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.remove_orphans(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn attach_logs_empty_attach_returns() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("atl");
	let engine = Engine::new(client, proj.clone());
	// attach: false — attach_logs finds no targets and returns immediately
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    attach: false\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.attach_logs(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn up_skips_recreate_when_config_unchanged() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("rch");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// Same config again -> config-hash matches -> skip recreate + ensure started.
	engine.up(&file).await.unwrap();
	// force_recreate -> recreate even though config is unchanged.
	engine
		.up_with_options(&file, false, &[], &[], false, true, false)
		.await
		.unwrap();
	// Changed config -> hash differs -> recreate.
	let file2 = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"120\"]\n",
	)
	.unwrap();
	engine.up(&file2).await.unwrap();
	engine.down(&file2).await.unwrap();
}
