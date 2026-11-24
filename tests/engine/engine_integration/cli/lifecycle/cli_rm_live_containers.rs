//! CLI binary integration tests for `rm` without `-f`, against Podman: a
//! container that is running or paused is skipped with a warning and the exit
//! status is 0, a container that has stopped or was only created is removed,
//! and one `rm` does both when a project has both.
use super::*;
use std::process::Command;

const SLEEP: &str = "image: alpine:latest\n    command: [\"sleep\", \"infinity\"]";

fn exists(name: &str) -> bool {
	Command::new("podman")
		.args(["container", "exists", name])
		.status()
		.expect("run podman container exists")
		.success()
}

fn project(tag: &str) -> super::DownGuard {
	super::DownGuard::new(
		tag,
		&format!("services:\n  live:\n    {SLEEP}\n  done:\n    {SLEEP}\n"),
	)
}

#[tokio::test]
async fn cli_rm_skips_a_running_container_and_removes_a_stopped_one() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = project("rmlive");
	let (c, proj) = (guard.compose_path(), guard.name());
	let (live, done) = (format!("{proj}-live-1"), format!("{proj}-done-1"));

	run_ok(&["-f", c, "-p", proj, "up", "-d"]);
	run_ok(&["-f", c, "-p", proj, "stop", "done"]);

	let out = run_ok(&["-f", c, "-p", proj, "rm"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains(&format!(
			"{live} is running; skipping (pass -f to force removal)"
		)),
		"{stderr}"
	);
	assert!(exists(&live), "a running container must stay");
	assert!(!exists(&done), "the stopped one is removed in the same run");
}

#[tokio::test]
async fn cli_rm_skips_a_paused_container() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = project("rmpause");
	let (c, proj) = (guard.compose_path(), guard.name());
	let live = format!("{proj}-live-1");

	run_ok(&["-f", c, "-p", proj, "up", "-d", "live"]);
	run_ok(&["-f", c, "-p", proj, "pause", "live"]);

	let out = run_ok(&["-f", c, "-p", proj, "rm", "live"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains(&format!(
			"{live} is paused; skipping (pass -f to force removal)"
		)),
		"{stderr}"
	);
	assert!(exists(&live), "a paused container must stay");
}

#[tokio::test]
async fn cli_rm_removes_a_container_that_was_only_created() {
	if super::podman().await.is_none() {
		return;
	}
	let guard = project("rmcreated");
	let (c, proj) = (guard.compose_path(), guard.name());
	let done = format!("{proj}-done-1");

	run_ok(&["-f", c, "-p", proj, "up", "--no-start", "done"]);
	assert!(exists(&done), "the container was created");
	run_ok(&["-f", c, "-p", proj, "rm", "done"]);
	assert!(!exists(&done), "a created container is removed without -f");
}
