//! Engine integration tests for the container options that are silently droppable:
//! `expose`, `ulimits` and `env_file`.
use std::fs;

use super::*;
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
	let cname = format!("{proj}-web-1");
	// Both keys are silently droppable: a container with neither still starts, and
	// `up` returns Ok either way. The ulimits are readable from inside the
	// container; `expose` is container config, so read it out of band.
	let limits = engine
		.test_exec_capture(
			&cname,
			vec!["sh".into(), "-c".into(), "ulimit -Sn; ulimit -Hn".into()],
		)
		.await
		.unwrap_or_default();
	let exposed = String::from_utf8_lossy(
		&std::process::Command::new("podman")
			.args([
				"inspect",
				&cname,
				"--format",
				"{{json .Config.ExposedPorts}}",
			])
			.output()
			.expect("podman inspect")
			.stdout,
	)
	.trim()
	.to_string();
	engine.down(&file).await.unwrap();

	assert_eq!(
		limits.trim(),
		"1024\n2048",
		"the soft and hard nofile limits did not reach the container"
	);
	// The `/tcp` suffix is the point: `expose: "8080/tcp"` takes the raw-string
	// branch, and a parser that dropped the protocol would still produce a port.
	assert!(
		exposed.contains("8080/tcp"),
		"expose did not reach the container config with its protocol: {exposed:?}"
	);
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
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    env_file:\n      - test.env\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// The variable has to reach the container's environment. Reaching the line
	// that loads the file is not the same thing: this test used to note which
	// source line it covered and then assert nothing, so it would have passed just
	// as well with `env_file` dropped on the floor.
	let read = engine
		.exec_with_options(
			&file,
			"web",
			vec![
				"sh".to_string(),
				"-c".to_string(),
				"test \"$MYVAR\" = hello".to_string(),
			],
			podup::ExecOptions::default(),
		)
		.await;
	engine.down(&file).await.unwrap();
	assert!(
		read.is_ok(),
		"MYVAR from env_file did not reach the container's environment: {read:?}"
	);
}
