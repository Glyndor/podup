//! CLI binary integration tests for `config` without a Podman, and for `up` and
//! `down` end to end.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[test]
fn cli_config_no_podman() {
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(&compose, "services:\n  web:\n    image: alpine:latest\n").unwrap();

	let out = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "config"])
		.output()
		.expect("podup binary not found");

	assert!(
		out.status.success(),
		"config failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(stdout.contains("alpine"));
}

#[tokio::test]
async fn cli_up_and_down_via_binary() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let pid = std::process::id();
	let proj = format!("t{}-cli", pid);
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let up = Command::new(bin())
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
	assert!(
		up.status.success(),
		"up failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);

	let down = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
	assert!(
		down.status.success(),
		"down failed: {}",
		String::from_utf8_lossy(&down.stderr)
	);
}
