//! Engine integration tests (split for the source line limit).
use super::*;

// Configs: file and environment sources
// ---------------------------------------------------------------------------

#[tokio::test]
async fn file_config_bound() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let cfg_file = dir.path().join("app.conf");
	fs::write(&cfg_file, b"key=from-file").unwrap();

	let proj = proj("fcfg");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let yaml = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    configs:\n      - filecfg\nconfigs:\n  filecfg:\n    file: {}\n",
		cfg_file.display()
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[test]
fn env_config_materialized() {
	let rt = tokio::runtime::Runtime::new().unwrap();
	temp_env::with_var("PODUP_TEST_CFG_VAR", Some("cfg-from-env"), || {
		rt.block_on(async {
			let client = match podman().await {
				Some(d) => d,
				None => return,
			};
			let proj = proj("ecfg");
			let engine = Engine::new(client, proj.clone());
			let file = parse_str(
				"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    configs:\n      - envcfg\nconfigs:\n  envcfg:\n    environment: PODUP_TEST_CFG_VAR\n",
			)
			.unwrap();

			engine.up(&file).await.unwrap();
			engine.down(&file).await.unwrap();
		});
	});
}

// ---------------------------------------------------------------------------
// Container options: expose, deploy labels, annotations, tmpfs long-form
// ---------------------------------------------------------------------------

#[tokio::test]
async fn service_with_expose_deploy_labels_annotations_tmpfs() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("sdl");
	let engine = Engine::new(client, proj.clone());
	// expose covers container.rs L56-63
	// deploy.labels are accepted but, per the Compose Specification, are set on
	// the service only and must not be applied to the container
	// annotations covers container.rs L81-82
	// long-form tmpfs volume covers container.rs L107-113 and L139
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    expose:\n      - \"8080\"\n    annotations:\n      com.example.note: value\n    deploy:\n      labels:\n        com.example.env: test\n    volumes:\n      - type: tmpfs\n        target: /tmp/cache\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Volume: named volume with driver_opts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn named_volume_with_driver_opts() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("vdo");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	// driver_opts covers volume.rs L55 (Some(driver_opts) branch)
	// Use a bind-mount volume pointing to the temp dir (fast, rootless-safe)
	let yaml = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    volumes:\n      - {proj}-cache:/cache\nvolumes:\n  {proj}-cache:\n    driver: local\n    driver_opts:\n      type: none\n      o: bind\n      device: {path}\n",
		path = dir.path().display()
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	engine.down_with_options(&file, true).await.unwrap();
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

#[tokio::test]
async fn build_with_target_stage() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	// Multi-stage Dockerfile — build with target: base covers build.rs L77
	fs::write(
		dir.path().join("Dockerfile"),
		b"FROM alpine:latest AS base\nRUN echo base-stage\nFROM base AS final\nRUN echo final-stage\n",
	)
	.unwrap();

	let proj = proj("bst");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let image_tag = format!("podup-test-bst-{}:latest", std::process::id());
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n      target: base\n    image: {image_tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn build_with_args_and_extra_tags() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("bat");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let pid = std::process::id();
	let main_tag = format!("podup-test-bat-{}:latest", pid);
	let extra_tag = format!("podup-test-bat-extra-{}:v1", pid);
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        ARG VERSION=0\n        RUN echo Version $VERSION\n      args:\n        VERSION: \"1.0\"\n      tags:\n        - {extra_tag}\n    image: {main_tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn build_with_cli_no_cache_and_build_arg() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("bco");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let tag = format!("podup-test-bco-{}:latest", std::process::id());
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        ARG VERSION=0\n        RUN echo Version $VERSION\n      args:\n        VERSION: \"1.0\"\n    image: {tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	// CLI overrides: force no-cache and override the compose VERSION build arg.
	engine
		.build_all_with_options(
			&file,
			&[],
			&podup::BuildOptions {
				no_cache: true,
				build_args: vec!["VERSION=2.0".to_string()],
				..Default::default()
			},
		)
		.await
		.unwrap();
}

#[tokio::test]
async fn build_inline_dockerfile() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("bld");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let image_tag = format!("podup-test-build-{}:latest", std::process::id());
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        RUN echo built\n    image: {image_tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn build_from_dockerfile_in_context() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("Dockerfile"),
		b"FROM alpine:latest\nRUN echo context-build\n",
	)
	.unwrap();

	let proj = proj("bdc");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let image_tag = format!("podup-test-build-ctx-{}:latest", std::process::id());
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n    image: {image_tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Networks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn explicit_network_created() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("net");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    networks:\n      - mynet\nnetworks:\n  mynet:\n    driver: bridge\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Secret/config long-form refs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn secret_long_form_ref() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("slf");
	let engine = Engine::new(client, proj.clone());
	// mode is octal notation per the Compose Specification (leading-zero `0400`);
	// uid is passed through to the native secret spec.
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets:\n      - source: mysecret\n        target: /run/secrets/custom_name\n        mode: 0400\n        uid: \"0\"\nsecrets:\n  mysecret:\n    content: \"topsecret\"\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine
		.exec_with_options(
			&file,
			"web",
			vec!["cat".to_string(), "/run/secrets/custom_name".to_string()],
			podup::ExecOptions::default(),
		)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

#[tokio::test]
async fn config_long_form_ref() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("clf");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    configs:\n      - source: mycfg\n        target: /etc/app.conf\nconfigs:\n  mycfg:\n    content: \"key=value\"\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine
		.exec_with_options(
			&file,
			"web",
			vec!["cat".to_string(), "/etc/app.conf".to_string()],
			podup::ExecOptions::default(),
		)
		.await
		.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// External volume skip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_volume_missing_errors_on_up() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("exv");
	let engine = Engine::new(client, proj.clone());
	// An external volume that does not exist must surface an error rather than
	// being silently skipped (compose spec requires the resource to exist).
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\nvolumes:\n  extdata-does-not-exist:\n    external: true\n",
	)
	.unwrap();

	let result = engine.up(&file).await;
	assert!(
		matches!(result, Err(podup::ComposeError::ExternalNotFound(_))),
		"expected ExternalNotFound, got {result:?}"
	);
}

// ---------------------------------------------------------------------------
// External (Podman-native) secret injection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_secret_missing_errors_on_up() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("exsec");
	let engine = Engine::new(client, proj.clone());
	// An `external: true` secret that no `podman secret` backs must fail closed,
	// like an external volume, rather than start a container missing the secret.
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets: [absent-secret]\nsecrets:\n  absent-secret:\n    external: true\n",
	)
	.unwrap();

	let result = engine.up(&file).await;
	assert!(
		matches!(result, Err(podup::ComposeError::ExternalNotFound(_))),
		"expected ExternalNotFound, got {result:?}"
	);
}

#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn external_secret_injected_into_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("insec");
	let secret_name = format!("{proj}-tok");

	// Create the backing Podman secret out-of-band — the external-secret idiom.
	// Skip the test if the podman CLI is unavailable (socket alone is not enough).
	let dir = tempfile::tempdir().unwrap();
	let secret_src = dir.path().join("tok");
	fs::write(&secret_src, b"native-secret-value").unwrap();
	let created = std::process::Command::new("podman")
		.args([
			"secret",
			"create",
			&secret_name,
			secret_src.to_str().unwrap(),
		])
		.status();
	match created {
		Ok(s) if s.success() => {}
		_ => return,
	}

	// The compose name is `tok` (→ /run/secrets/tok); the actual secret is named
	// differently, exercising the source/target split.
	let yaml = format!(
		"services:\n  app:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets: [tok]\nsecrets:\n  tok:\n    external: true\n    name: {secret_name}\n"
	);
	let file = parse_str(&yaml).unwrap();
	let engine = Engine::new(client, proj.clone());
	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-app-1");
	let out = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/run/secrets/tok".into()])
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();
	let _ = std::process::Command::new("podman")
		.args(["secret", "rm", &secret_name])
		.status();

	assert!(
		out.contains("native-secret-value"),
		"external secret was not injected at /run/secrets/tok: {out:?}"
	);
}

// ---------------------------------------------------------------------------
// Orphan removal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remove_orphans_removes_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("orr");
	let engine = Engine::new(client, proj.clone());

	let file_svc1 = parse_str(
		"services:\n  svc1:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.up(&file_svc1).await.unwrap();

	// file_svc2 only declares svc2 — svc1 becomes an orphan
	let file_svc2 = parse_str(
		"services:\n  svc2:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	engine.remove_orphans(&file_svc2).await.unwrap();

	// cleanup (svc1 already removed; down() on either file is a no-op for missing containers)
	let _ = engine.down(&file_svc1).await;
}

// ---------------------------------------------------------------------------
