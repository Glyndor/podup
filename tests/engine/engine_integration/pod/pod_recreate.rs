//! `x-podman-pod: true` against a real Podman: a changed port set recreates the pod.
use std::process::Command;

use super::pod_fixtures::{down, pod_json, write_compose};
use super::*;
#[tokio::test]
async fn a_port_change_recreates_the_pod() {
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("pod-port");
	let compose = write_compose(dir.path(), "");
	let first = Command::new(bin())
		.args(["-f", &compose, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();
	let id_before = pod_json(&proj).and_then(|p| p["Id"].as_str().map(str::to_string));
	let compose2 = write_compose(dir.path(), "    ports:\n      - \"127.0.0.1:0:5432\"\n");
	let second = Command::new(bin())
		.args(["-f", &compose2, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();
	let id_after = pod_json(&proj).and_then(|p| p["Id"].as_str().map(str::to_string));
	let stdout = format!(
		"{}{}",
		String::from_utf8_lossy(&second.stdout),
		String::from_utf8_lossy(&second.stderr)
	);
	down(&compose2, &proj);
	assert!(
		first.status.success(),
		"first up failed: {}",
		String::from_utf8_lossy(&first.stderr)
	);
	assert!(
		second.status.success(),
		"second up failed: {}",
		String::from_utf8_lossy(&second.stderr)
	);
	assert!(
		id_before.is_some() && id_after.is_some(),
		"the pod must exist after both ups"
	);
	assert_ne!(
		id_before, id_after,
		"a changed port set must recreate the pod"
	);
	assert!(
		stdout.contains("Pod") && stdout.contains("Recreated"),
		"the recreate must be reported with the Recreated vocabulary:\n{stdout}"
	);
	assert!(
		!stdout.contains("orphan"),
		"the pod's infra container is not an orphan of the project:\n{stdout}"
	);
}
