//! CLI binary integration tests for `--wait`, on `up` and on `start`.
use super::*;
use std::fs;
use tempfile::tempdir;
#[tokio::test]
async fn cli_up_wait_returns_when_healthy() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-upwait", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	// A healthcheck that omits `timeout` (defaults applied) must still reach
	// healthy, so `up --wait` returns successfully instead of timing out.
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    healthcheck:\n      test: [\"CMD\", \"true\"]\n      interval: 1s\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let up = run(&["-f", c, "-p", &proj, "up", "-d", "--wait"]);
	assert!(up.status.success(), "up --wait failed: {:?}", up.stderr);
	run(&["-f", c, "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_start_wait_returns_after_starting() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-startwait", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	// Create the container without starting, then `start --wait` must start it
	// and return (no healthcheck → ready once started).
	run_ok(&["-f", c, "-p", &proj, "up", "--no-start"]);
	let start = run(&[
		"-f",
		c,
		"-p",
		&proj,
		"start",
		"--wait",
		"--wait-timeout",
		"30",
	]);
	assert!(
		start.status.success(),
		"start --wait failed: {:?}",
		start.stderr
	);

	run(&["-f", c, "-p", &proj, "down"]);
}
