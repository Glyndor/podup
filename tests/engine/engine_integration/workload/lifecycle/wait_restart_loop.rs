//! `up -d --wait` for a service with no healthcheck.
//!
//! A service in a crash loop is `running` whenever the wait happens to look, so
//! a wait that reads the state once says it is ready. Against a real Podman:
//! the loop fails the wait, a one-shot that ends well does not, a service that
//! stays up does not, and a one-shot that failed does.

use std::fs;

use super::*;

/// Run `up -d --wait` on a one-service project and hand back what it did.
fn wait_on(tag: &str, command: &str, restart: &str) -> std::process::Output {
	let dir = tempfile::tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let project = format!("t{}-{tag}", std::process::id());
	fs::write(
		&compose,
		format!(
			"services:\n  s:\n    image: busybox:latest\n    command: [\"sh\", \"-c\", \"{command}\"]\n    restart: {restart}\n"
		),
	)
	.unwrap();
	let file = compose.to_str().unwrap();
	let out = run(&["-f", file, "-p", &project, "up", "-d", "--wait"]);
	run(&["-f", file, "-p", &project, "down", "-t", "0"]);
	out
}

fn stderr(out: &std::process::Output) -> String {
	String::from_utf8_lossy(&out.stderr).into_owned()
}

#[tokio::test]
async fn a_crash_loop_does_not_pass_the_wait() {
	if super::podman().await.is_none() {
		return;
	}
	// Alive for a second, then dead, then started again by the policy.
	let out = wait_on("waitloop", "sleep 1; exit 3", "unless-stopped");
	assert!(
		!out.status.success(),
		"a crash loop passed: {}",
		stderr(&out)
	);
	assert!(
		stderr(&out).contains("keeps restarting") || stderr(&out).contains("exited with code 3"),
		"{}",
		stderr(&out)
	);
}

#[tokio::test]
async fn a_service_that_stays_up_passes() {
	if super::podman().await.is_none() {
		return;
	}
	let out = wait_on("waitup", "sleep 300", "unless-stopped");
	assert!(out.status.success(), "{}", stderr(&out));
}

#[tokio::test]
async fn a_one_shot_that_ends_well_passes() {
	if super::podman().await.is_none() {
		return;
	}
	let out = wait_on("waitone", "exit 0", "\"no\"");
	assert!(out.status.success(), "{}", stderr(&out));
}

#[tokio::test]
async fn a_one_shot_that_fails_does_not_pass() {
	if super::podman().await.is_none() {
		return;
	}
	let out = wait_on("waitfail", "exit 3", "\"no\"");
	assert!(!out.status.success(), "a failed one-shot passed");
	assert!(
		stderr(&out).contains("exited with code 3"),
		"{}",
		stderr(&out)
	);
}
