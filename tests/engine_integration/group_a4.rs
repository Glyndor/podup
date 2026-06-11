//! Engine integration tests (split for the source line limit).
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
// Profiles: COMPOSE_PROFILES env var path
// ---------------------------------------------------------------------------

#[test]
fn active_profiles_via_env() {
	let rt = tokio::runtime::Runtime::new().unwrap();
	// Set COMPOSE_PROFILES so active_profiles_set reads it (covers profiles.rs L15-19)
	temp_env::with_var("COMPOSE_PROFILES", Some("prod"), || {
		rt.block_on(async {
			let client = match podman().await {
				Some(d) => d,
				None => return,
			};
			let proj = proj("apv");
			let engine = Engine::new(client, proj.clone());
			// "debug" service has profile "debug" — not in "prod" → skipped
			let file = parse_str(
				"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  debug:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    profiles: [\"debug\"]\n",
			)
			.unwrap();
			// Pass empty active_profiles slice so it falls back to COMPOSE_PROFILES env
			engine
				.up_with_options(&file, false, &[], &[], false, false, false)
				.await
				.unwrap();
			engine.down(&file).await.unwrap();
		});
	});
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
	// init sleeps 1.5s before exiting; first poll sees it running (L73-75 covered)
	let file = parse_str(
		"services:\n  init:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"sleep 1.5; exit 0\"]\n  app:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      init:\n        condition: service_completed_successfully\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// External config skipped
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_config_skipped() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("xcfg");
	let engine = Engine::new(client, proj.clone());
	// Covers volume.rs L215 (external config debug log)
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    configs:\n      - extcfg\nconfigs:\n  extcfg:\n    external: true\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Container options: expose with slash, env_file, ulimits
// ---------------------------------------------------------------------------

#[tokio::test]
async fn service_with_expose_proto_and_ulimits() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	// expose "8080/tcp" (with slash) covers container.rs L57 (raw.clone() branch)
	// ulimits covers container.rs L150 (Some(ulimits))
	let proj = proj("seu");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    expose:\n      - \"8080/tcp\"\n    ulimits:\n      nofile:\n        soft: 1024\n        hard: 2048\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn env_file_loaded() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	fs::write(dir.path().join("test.env"), b"MYVAR=hello\n").unwrap();

	let proj = proj("evf");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	// env_file covers container.rs L278 (load_env_file_entries)
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    env_file:\n      - test.env\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Lifecycle: target_set skips non-targeted service; dep profile skip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn target_services_skips_non_dep() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("tsk");
	let engine = Engine::new(client, proj.clone());
	// "extra" is not depended upon by web → skipped (lifecycle.rs L56 continue)
	let file = parse_str(
		"services:\n  extra:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine
		.up_with_options(&file, false, &[], &["web".to_string()], false, false, false)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn dep_on_profile_filtered_service() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("dpf");
	let engine = Engine::new(client, proj.clone());
	// "db" has profile "debug" → not active → dep wait skipped (lifecycle.rs L73)
	// "web" depends on "db" but db is profile-filtered so its dep wait is skipped
	let file = parse_str(
		"services:\n  db:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    profiles: [\"debug\"]\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      - db\n",
	)
	.unwrap();

	// No active profiles → db is skipped; web still runs but skips db's dep wait
	engine
		.up_with_options(&file, false, &[], &[], false, false, false)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Build: arg with null value (from environment)
// ---------------------------------------------------------------------------

#[test]
fn build_with_env_arg() {
	let rt = tokio::runtime::Runtime::new().unwrap();
	// FROM_ENV has no explicit value → read from environment (build.rs L89 None branch)
	temp_env::with_var("FROM_ENV", Some("test-value"), || {
		rt.block_on(async {
			let client = match podman().await {
				Some(d) => d,
				None => return,
			};
			let dir = tempfile::tempdir().unwrap();
			let proj = proj("bea");
			let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
			let image_tag = format!("podup-test-bea-{}:latest", std::process::id());
			let yaml = format!(
				"services:\n  app:\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        ARG FROM_ENV\n        RUN echo env=$FROM_ENV\n      args:\n        FROM_ENV:\n    image: {image_tag}\n    command: [\"sleep\", \"infinity\"]\n"
			);
			let file = parse_str(&yaml).unwrap();

			engine.up(&file).await.unwrap();
			engine.down(&file).await.unwrap();
		});
	});
}

// ---------------------------------------------------------------------------
// label_file: load labels from file (container.rs L73-74)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn label_file_labels_applied() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("svc.labels"),
		b"com.example.role=web\ncom.example.env=test\n",
	)
	.unwrap();
	let proj = proj("lfl");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    label_file: svc.labels\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// optional dep not in file (lifecycle.rs L45, L70)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn optional_dep_not_in_file() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("odf");
	let engine = Engine::new(client, proj.clone());
	// ghost_db not in services + required:false → resolve_order skips it,
	// target_set pushes it (file.services.get → None → L45),
	// dep-wait loop hits None => continue (L70)
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      ghost_db:\n        condition: service_started\n        required: false\n",
	)
	.unwrap();

	engine
		.up_with_options(&file, false, &[], &["web".to_string()], false, false, false)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// duplicate target_services triggers continue in target_set (lifecycle.rs L37)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn target_services_duplicate_entry() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("tde");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	// Passing "web" twice causes it to be pushed to the target_set stack twice;
	// the second pop finds "web" already in the set → !set.insert → continue (L37).
	engine
		.up_with_options(
			&file,
			false,
			&[],
			&["web".to_string(), "web".to_string()],
			false,
			false,
			false,
		)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}
