//! CLI binary integration tests for `rm`, and for `pause` and `unpause`.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_rm_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	// `up` creates the project network, `stop` only halts containers, and `rm`
	// removes containers. None of them removes `<project>_default`, so without
	// a guard a passing test or a panicked one leaves the network on the host
	// to be picked up by the leak scanner on the next run.
	let _down = super::DownGuard::new(
		"clrm",
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	);
	let c = _down.compose_path();
	let proj = _down.name();

	Command::new(bin())
		.args(["-f", c, "-p", proj, "up", "--detach"])
		.output()
		.unwrap();

	Command::new(bin())
		.args(["-f", c, "-p", proj, "stop"])
		.output()
		.unwrap();

	let rm = Command::new(bin())
		.args(["-f", c, "-p", proj, "rm"])
		.output()
		.unwrap();
	assert!(
		rm.status.success(),
		"rm failed: {}",
		String::from_utf8_lossy(&rm.stderr)
	);
}

#[tokio::test]
async fn cli_pause_unpause_subcommands() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clpu", std::process::id());
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

	let pause = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "pause"])
		.output()
		.unwrap();
	assert!(
		pause.status.success(),
		"pause failed: {}",
		String::from_utf8_lossy(&pause.stderr)
	);

	let unpause = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "unpause"])
		.output()
		.unwrap();
	assert!(
		unpause.status.success(),
		"unpause failed: {:?}",
		unpause.stderr
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}
