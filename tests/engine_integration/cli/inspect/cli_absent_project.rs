//! CLI binary integration tests for the commands run against a project that was
//! never created or never started.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
/// #758: `down` on a defined-but-never-created project is a clean, quiet no-op:
/// it must not synthesize predicted container names and leak a raw 404 / "could
/// not stop" warning.
#[tokio::test]
async fn cli_down_on_never_created_is_quiet_noop() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-dnvr", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	// `down` without a prior `up`: nothing exists, so this must exit 0 cleanly.
	let down = Command::new(bin())
		.env("RUST_LOG", "info")
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
	assert!(
		down.status.success(),
		"down on never-created failed: {}",
		String::from_utf8_lossy(&down.stderr)
	);
	let stderr = String::from_utf8_lossy(&down.stderr);
	// A libpod error line, not a bare "404": the project name carries the
	// pid, which can contain "404" (#1977).
	assert_no_libpod_http_error(&stderr);
	assert!(
		!stderr.contains("no such container"),
		"down leaked a libpod `no such container` for a never-created project: {stderr}"
	);
	assert!(
		!stderr.contains("could not stop"),
		"down warned about stopping a phantom container: {stderr}"
	);
}

/// #758: `wait` on a defined-but-never-created service returns cleanly instead of
/// surfacing a raw `podman API error (HTTP 404)`.
#[tokio::test]
async fn cli_wait_on_never_created_is_quiet_noop() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-wnvr", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let wait = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "wait", "web"])
		.output()
		.unwrap();
	assert!(
		wait.status.success(),
		"wait on never-created failed: {}",
		String::from_utf8_lossy(&wait.stderr)
	);
	let stderr = String::from_utf8_lossy(&wait.stderr);
	// Same check as `cli_down_on_never_created_is_quiet_noop` (#1977).
	assert_no_libpod_http_error(&stderr);
}

/// #876: `stop` on a Created (never-started) container must not claim it was
/// "stopped": it is a harmless no-op, so no "stopped" line is logged.
#[tokio::test]
async fn cli_stop_on_created_does_not_report_stopped() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-screa", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	// `create` builds the container but never starts it (state = Created).
	let create = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "create"])
		.output()
		.unwrap();
	assert!(
		create.status.success(),
		"create failed: {}",
		String::from_utf8_lossy(&create.stderr)
	);

	// `stop` with INFO logging on: a not-running container must not be reported
	// as "stopped".
	let stop = Command::new(bin())
		.env("RUST_LOG", "info")
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "stop"])
		.output()
		.unwrap();
	assert!(
		stop.status.success(),
		"stop failed: {}",
		String::from_utf8_lossy(&stop.stderr)
	);
	let stderr = String::from_utf8_lossy(&stop.stderr);
	assert!(
		!stderr.contains("stopped"),
		"stop claimed it stopped a not-running container: {stderr}"
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}
