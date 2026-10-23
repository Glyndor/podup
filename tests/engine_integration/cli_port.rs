//! CLI binary integration tests for `port`.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_port_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clprt", std::process::id());
	// A port chosen at run time, not a constant: three tests shared 18081 and a
	// fourth 18080, so any two running at once lost the bind and failed with
	// `pasta failed ... Address already in use`.
	let port = super::free_port();
	fs::write(
		&compose,
		format!(
			"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    ports:\n      - \"127.0.0.1:{port}:80\"\n"
		),
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

	let port = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"port",
			"web",
			"80",
		])
		.output()
		.unwrap();
	assert!(port.status.success(), "port failed: {:?}", port.stderr);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}
