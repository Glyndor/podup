//! `up --wait --wait-timeout` is the whole budget of the wait.
//!
//! A service's own budget is `interval x retries + start_period`, which for a
//! check that runs every second and may fail twice is two seconds. A
//! `--wait-timeout` of forty seconds used to leave that two seconds in place, so
//! a service that became healthy at seven was reported as a health-check
//! timeout at two, with the forty on the command line unread. `start --wait`
//! already extended it; `up --wait` now does too.
//!
//! `docker compose up --wait --wait-timeout` does not extend it: it fails the
//! wait as soon as the runtime marks the container `unhealthy` (measured against
//! the same Podman 6.2.0 socket: 1.8 s with the same two retries), and caps the
//! wait at the timeout when the check is still failing but has retries left
//! (`application not healthy after 5s`, 5.4 s).

use std::fs;
use std::time::Instant;

use super::*;

/// A service healthy about seven seconds after it starts, whose check may fail
/// twice at one-second intervals before its own budget is spent.
fn slow(project: &str, healthy_after: &str) -> (tempfile::TempDir, String, String) {
	let dir = tempfile::tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		format!(
			"services:\n  web:\n    image: busybox:latest\n    command: [\"sh\", \"-c\", \"{healthy_after}; sleep 300\"]\n    healthcheck:\n      test: [\"CMD-SHELL\", \"test -f /tmp/ready\"]\n      interval: 1s\n      timeout: 1s\n      retries: 2\n"
		),
	)
	.unwrap();
	let file = compose.to_str().unwrap().to_string();
	(dir, file, format!("t{}-{project}", std::process::id()))
}

fn stderr(out: &std::process::Output) -> String {
	String::from_utf8_lossy(&out.stderr).into_owned()
}

#[tokio::test]
async fn the_wait_timeout_extends_the_budget_of_a_slow_service() {
	if super::podman().await.is_none() {
		return;
	}
	let (_dir, file, project) = slow("waitslow", "sleep 6; touch /tmp/ready");
	let out = run(&[
		"-f",
		&file,
		"-p",
		&project,
		"up",
		"-d",
		"--wait",
		"--wait-timeout",
		"40",
	]);
	run(&["-f", &file, "-p", &project, "down", "-t", "0"]);
	assert!(out.status.success(), "{}", stderr(&out));
}

#[tokio::test]
async fn without_a_wait_timeout_the_services_own_budget_still_decides() {
	if super::podman().await.is_none() {
		return;
	}
	let (_dir, file, project) = slow("waitplain", "sleep 6; touch /tmp/ready");
	let out = run(&["-f", &file, "-p", &project, "up", "-d", "--wait"]);
	run(&["-f", &file, "-p", &project, "down", "-t", "0"]);
	assert!(!out.status.success(), "the two-second budget did not hold");
	assert!(
		stderr(&out).contains("health check timeout"),
		"{}",
		stderr(&out)
	);
}

#[tokio::test]
async fn a_check_that_never_passes_ends_at_the_wait_timeout() {
	if super::podman().await.is_none() {
		return;
	}
	let (_dir, file, project) = slow("waitnever", "true");
	let started = Instant::now();
	let out = run(&[
		"-f",
		&file,
		"-p",
		&project,
		"up",
		"-d",
		"--wait",
		"--wait-timeout",
		"6",
	]);
	let took = started.elapsed();
	run(&["-f", &file, "-p", &project, "down", "-t", "0"]);
	assert!(!out.status.success());
	assert!(
		stderr(&out).contains("timed out after 6s"),
		"{}",
		stderr(&out)
	);
	assert!(
		took.as_secs() >= 5 && took.as_secs() < 20,
		"the wait ended after {took:?}, not at the 6 seconds asked for"
	);
}
