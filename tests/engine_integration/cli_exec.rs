//! CLI binary integration tests for `exec`.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_exec_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clex", std::process::id());
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

	let exec = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"exec",
			"web",
			"echo",
			"cli-exec",
		])
		.output()
		.unwrap();
	assert!(exec.status.success());
	// The command was chosen to print something and then nobody read it.
	let out = String::from_utf8_lossy(&exec.stdout);
	assert_eq!(
		out.trim(),
		"cli-exec",
		"exec did not forward the command's output"
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}
