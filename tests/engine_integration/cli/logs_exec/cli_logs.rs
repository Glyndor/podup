//! CLI binary integration tests for `logs`.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_logs_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-cllg", std::process::id());
	// The container has to say something. With `sleep infinity` it prints nothing,
	// so `logs` has no output to be right or wrong about and no assertion on it
	// could fail.
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo hello-from-web; sleep infinity\"]\n",
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

	let logs = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "logs"])
		.output()
		.unwrap();
	assert!(logs.status.success());
	// Both halves of the line are a contract. The container's own output is the
	// point of the command, and the `service |` prefix was missing entirely once
	// (#594), which is invisible to a check on the exit code.
	let out = String::from_utf8_lossy(&logs.stdout);
	assert!(
		out.contains("hello-from-web"),
		"logs did not carry the container's output: {out:?}"
	);
	assert!(
		out.contains("web-1 |"),
		"logs dropped the per-service prefix: {out:?}"
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}
