//! CLI binary integration tests for `cp` out of a container.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_cp_from_container() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clcp", std::process::id());
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

	let dst = dir.path().to_str().unwrap();
	let cp = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"cp",
			"web:/etc/hostname",
			dst,
		])
		.output()
		.unwrap();
	assert!(cp.status.success(), "cp failed: {:?}", cp.stderr);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}
