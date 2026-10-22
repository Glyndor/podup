//! Engine integration tests for the project resources a service can mount:
//! named volumes, and the secrets and configs podup creates for them.
use std::fs;

use super::*;
// Seven assertions in this suite deliberately do not print the `Result` they
// assert on, while others still do. `rust/cleartext-logging` takes its sources
// from the NAME of the called function, so `create_project_secrets` is a taint
// source and those seven were the sinks it reached through `?`. Nothing
// syntactic separates them from the rest, so this note is what keeps the
// inconsistency from reading as an oversight somebody tidies up. #1599.

// Volumes, secrets, configs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn named_volume_created_on_up() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("nvol");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(&format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    volumes:\n      - {proj}-data:/data\nvolumes:\n  {proj}-data:\n"
	))
	.unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	// Write into the mount, then force a recreate. A named volume outlives the
	// container that mounted it, so the file has to still be there afterwards. If
	// /data were ordinary container filesystem, the recreate would take it along,
	// and `up` returning Ok would look exactly the same either way.
	//
	// Known limit, measured rather than assumed: this does NOT detect podup
	// skipping the volume creation. With `create_volumes` stubbed out to a no-op
	// the test still passed, because Podman creates a named volume on first use
	// anyway. What explicit creation adds is the `podup.project` label that makes
	// `down --volumes` recognise the volume as the project's, and reading a
	// volume's labels is not something the library returns. Anyone tightening
	// this needs that, not another assertion on the contents.
	engine
		.test_exec_capture(
			&cname,
			vec![
				"sh".into(),
				"-c".into(),
				"echo in-volume > /data/marker".into(),
			],
		)
		.await
		.unwrap();
	engine
		.up_with_options(&file, false, &[], &[], false, true, false, false)
		.await
		.unwrap();
	let after = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/data/marker".into()])
		.await
		.unwrap_or_default();
	engine.down_with_options(&file, true).await.unwrap();

	assert_eq!(
		after.trim(),
		"in-volume",
		"the mount did not survive a container recreate, so it was not backed by the named volume"
	);
}

#[tokio::test]
async fn inline_secret_materialized() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("sec");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets:\n      - mysecret\nsecrets:\n  mysecret:\n    content: \"supersecret\"\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// Inline content is created as a Podman-native secret and mounted at the
	// usual /run/secrets/<name> path. `cat` alone only proves the path exists
	// (it exits 0 on an empty file too) so compare the bytes.
	let read = engine
		.exec_with_options(
			&file,
			"web",
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"test \"$(cat /run/secrets/mysecret)\" = supersecret".to_string(),
			],
			podup::ExecOptions::default(),
		)
		.await;
	engine.down(&file).await.unwrap();
	assert!(
		read.is_ok(),
		"the inline secret did not reach /run/secrets/mysecret with its content: {read:?}"
	);
}

#[tokio::test]
async fn file_secret_bound() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let secret_file = dir.path().join("my_secret.txt");
	fs::write(&secret_file, b"file-secret-content").unwrap();

	let proj = proj("fsec");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let yaml = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets:\n      - filesecret\nsecrets:\n  filesecret:\n    file: {}\n",
		secret_file.display()
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	// The container must READ the secret, not merely be started with it attached.
	// Asserting only that `up` and `down` succeed is what let a `file:` secret stay
	// unreadable on every SELinux-enforcing host for the life of the feature: the
	// mount was there, at the right path, with the right bytes behind it, and the
	// container was denied the open. `run` propagates the command's exit code, so
	// the read is checked without a sleep standing in for synchronisation.
	let read = engine
		.run(
			&file,
			"web",
			podup::RunOptions::new(
				vec![
					"sh".to_string(),
					"-c".to_string(),
					"test \"$(cat /run/secrets/filesecret)\" = file-secret-content".to_string(),
				],
				true,
				false,
				Vec::new(),
				None,
				false,
			),
		)
		.await;
	engine.down(&file).await.unwrap();
	assert!(
		read.is_ok(),
		"the container could not read /run/secrets/filesecret"
	);
}

#[test]
fn env_secret_materialized() {
	let rt = tokio::runtime::Runtime::new().unwrap();
	temp_env::with_var("PODUP_TEST_SECRET_VAR", Some("env-secret-value"), || {
		rt.block_on(async {
			let client = match podman().await {
				Some(d) => d,
				None => return,
			};
			let proj = proj("esec");
			let engine = Engine::new(client, proj.clone());
			let file = parse_str(
				"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets:\n      - envsecret\nsecrets:\n  envsecret:\n    environment: PODUP_TEST_SECRET_VAR\n",
			)
			.unwrap();

			engine.up(&file).await.unwrap();
			// The point of an `environment:` source is that the variable's value
			// becomes the secret. Starting the container proves neither half.
			let read = engine
				.exec_with_options(
					&file,
					"web",
					vec![
						"sh".to_string(),
						"-c".to_string(),
						"test \"$(cat /run/secrets/envsecret)\" = env-secret-value".to_string(),
					],
					podup::ExecOptions::default(),
				)
				.await;
			engine.down(&file).await.unwrap();
			assert!(
				read.is_ok(),
				"the env-sourced secret did not carry the variable's value: {read:?}"
			);
		});
	});
}

#[tokio::test]
async fn invalid_secret_name_rejected() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("isec");
	let engine = Engine::new(client, proj.clone());
	// A traversal name must be refused: it would otherwise become part of a
	// project-scoped Podman secret name and a URL query parameter.
	//
	// This test used to declare a perfectly ordinary secret called `evils`, discard
	// both results with `let _ =`, and say in a comment that it could not test the
	// thing its name promises. It asserted nothing at all, in either direction.
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets:\n      - '../evil'\nsecrets:\n  '../evil':\n    content: bad\n",
	)
	.unwrap();

	let result = engine.up(&file).await;
	let _ = engine.down(&file).await;
	assert!(
		result.is_err(),
		"a secret named '../evil' was accepted instead of being rejected"
	);
}

#[tokio::test]
async fn inline_config_materialized() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("cfg");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    configs:\n      - mycfg\nconfigs:\n  mycfg:\n    content: \"key=value\"\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// A config defaults to an absolute container-root path, unlike a secret's
	// /run/secrets/<name>, so this also pins the default target, not just the
	// content.
	let read = engine
		.exec_with_options(
			&file,
			"web",
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"test \"$(cat /mycfg)\" = key=value".to_string(),
			],
			podup::ExecOptions::default(),
		)
		.await;
	engine.down(&file).await.unwrap();
	assert!(
		read.is_ok(),
		"the inline config did not reach /mycfg with its content: {read:?}"
	);
}
