//! The parse-time port-exposure warning only belongs on commands that
//! create or install containers, plus `config` (#2020).

use std::fs;
use std::process::Command;

const PORT_EXPOSURE_NEEDLE: &str = "is published on every interface";
const PORT_EXPOSED_COMPOSE: &str = "\
services:
  web:
    image: alpine:3.19
    ports: [\"5432:5432\"]
";

/// The warning fires during parsing, before the unreachable socket stops
/// commands that need a daemon. Only stderr matters for these assertions.
fn run_podup_isolated(args: &[&str]) -> String {
	let dir = tempfile::tempdir().expect("create temp dir");
	let file = dir.path().join("compose.yaml");
	fs::write(&file, PORT_EXPOSED_COMPOSE).expect("write compose file");
	let out = Command::new(env!("CARGO_BIN_EXE_podup"))
		.arg("-f")
		.arg(&file)
		.args(args)
		.env_remove("RUST_LOG")
		.env(
			"PODMAN_SOCKET",
			"/nonexistent/podup-port-warning-scope.sock",
		)
		.output()
		.expect("run podup");
	String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn restart_does_not_warn() {
	let stderr = run_podup_isolated(&["restart"]);
	assert!(!stderr.contains(PORT_EXPOSURE_NEEDLE), "restart: {stderr}");
}

#[test]
fn stop_does_not_warn() {
	let stderr = run_podup_isolated(&["stop"]);
	assert!(!stderr.contains(PORT_EXPOSURE_NEEDLE), "stop: {stderr}");
}

#[test]
fn start_does_not_warn() {
	let stderr = run_podup_isolated(&["start"]);
	assert!(!stderr.contains(PORT_EXPOSURE_NEEDLE), "start: {stderr}");
}

#[test]
fn exec_does_not_warn() {
	let stderr = run_podup_isolated(&["exec", "web", "true"]);
	assert!(
		!stderr.contains(PORT_EXPOSURE_NEEDLE),
		"exec web true: {stderr}"
	);
}

#[test]
fn pause_does_not_warn() {
	let stderr = run_podup_isolated(&["pause"]);
	assert!(!stderr.contains(PORT_EXPOSURE_NEEDLE), "pause: {stderr}");
}

#[test]
fn unpause_does_not_warn() {
	let stderr = run_podup_isolated(&["unpause"]);
	assert!(!stderr.contains(PORT_EXPOSURE_NEEDLE), "unpause: {stderr}");
}

#[test]
fn kill_does_not_warn() {
	let stderr = run_podup_isolated(&["kill"]);
	assert!(!stderr.contains(PORT_EXPOSURE_NEEDLE), "kill: {stderr}");
}

#[test]
fn down_does_not_warn() {
	let stderr = run_podup_isolated(&["down"]);
	assert!(!stderr.contains(PORT_EXPOSURE_NEEDLE), "down: {stderr}");
}

#[test]
fn rm_does_not_warn() {
	let stderr = run_podup_isolated(&["rm", "-f"]);
	assert!(!stderr.contains(PORT_EXPOSURE_NEEDLE), "rm -f: {stderr}");
}

#[test]
fn events_does_not_warn() {
	let stderr = run_podup_isolated(&["events"]);
	assert!(!stderr.contains(PORT_EXPOSURE_NEEDLE), "events: {stderr}");
}

#[test]
fn commands_that_create_containers_warn() {
	for args in [
		&["up"][..],
		&["create"],
		&["run", "--rm", "web", "true"],
		&["scale", "web=1"],
	] {
		let stderr = run_podup_isolated(args);
		assert!(stderr.contains(PORT_EXPOSURE_NEEDLE), "{args:?}: {stderr}");
	}
}

#[test]
fn no_warn_silences_commands_that_create_containers() {
	for args in [
		&["--no-warn", "up"][..],
		&["--no-warn", "create"],
		&["--no-warn", "run", "--rm", "web", "true"],
		&["--no-warn", "scale", "web=1"],
	] {
		let stderr = run_podup_isolated(args);
		assert!(!stderr.contains(PORT_EXPOSURE_NEEDLE), "{args:?}: {stderr}");
	}
}
