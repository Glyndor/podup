//! CLI binary integration tests for `restart`, `stop`, `start` and `kill`.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_restart_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clrs", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"up",
			"--detach",
		])
		.output()
		.unwrap();

	let restart = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "restart"])
		.output()
		.unwrap();
	assert!(restart.status.success());
	// Progress goes to stderr, measured rather than assumed: asserting stdout here
	// would fail with nothing wrong. Six lifecycle commands used to exit 0 in
	// silence on a project that was never created, so naming the container is the
	// part worth pinning.
	let err = String::from_utf8_lossy(&restart.stderr);
	assert!(
		err.contains(&format!("{proj}-web-1")) && err.contains("Restarted"),
		"restart did not report which container it restarted: {err:?}"
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}

#[tokio::test]
async fn cli_stop_and_start_subcommands() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clss", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"up",
			"--detach",
		])
		.output()
		.unwrap();

	let stop = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "stop"])
		.output()
		.unwrap();
	assert!(
		stop.status.success(),
		"stop failed: {}",
		String::from_utf8_lossy(&stop.stderr)
	);

	let start = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "start"])
		.output()
		.unwrap();
	assert!(
		start.status.success(),
		"start failed: {}",
		String::from_utf8_lossy(&start.stderr)
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}

#[tokio::test]
async fn cli_kill_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clkl", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"up",
			"--detach",
		])
		.output()
		.unwrap();

	let kill = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"kill",
			"--signal",
			"SIGTERM",
		])
		.output()
		.unwrap();
	assert!(
		kill.status.success(),
		"kill failed: {}",
		String::from_utf8_lossy(&kill.stderr)
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}
