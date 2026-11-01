//! CLI binary integration tests for `run`.
use super::*;
use std::process::Command;
#[tokio::test]
async fn cli_run_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	// `podup run` builds the project network the same way `docker compose run`
	// does, then leaves it behind: there is no implicit teardown step. The
	// guard's `down -v` runs from drop, so a panic in either assertion still
	// reaps the network.
	let _down = super::DownGuard::new("clrun", "services:\n  job:\n    image: alpine:latest\n");
	let c = _down.compose_path();
	let proj = _down.name();

	let run = Command::new(bin())
		.args(["-f", c, "-p", proj, "run", "job", "echo", "hello"])
		.output()
		.unwrap();
	assert!(
		run.status.success(),
		"run failed: {}",
		String::from_utf8_lossy(&run.stderr)
	);
	let stdout = String::from_utf8_lossy(&run.stdout);
	assert!(stdout.contains("hello"), "expected 'hello' in output");
}

#[tokio::test]
async fn cli_run_nonzero_exit_propagates() {
	if super::podman().await.is_none() {
		return;
	}
	let _down = super::DownGuard::new("clrxc", "services:\n  job:\n    image: alpine:latest\n");
	let c = _down.compose_path();
	let proj = _down.name();

	let run = Command::new(bin())
		.args(["-f", c, "-p", proj, "run", "job", "false"])
		.output()
		.unwrap();
	assert!(!run.status.success(), "expected non-zero exit from 'false'");
	assert_eq!(run.status.code(), Some(1));
}
