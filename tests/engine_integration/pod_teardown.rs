//! `x-podman-pod: true` against a real Podman: `down` removes the pod.
use std::process::Command;

use super::pod_fixtures::{pod_json, write_compose};
use super::*;
#[tokio::test]
async fn down_removes_the_pod() {
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("pod-down");
	let compose = write_compose(dir.path(), "");
	let up = Command::new(bin())
		.args(["-f", &compose, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();
	let existed = pod_json(&proj).is_some();
	let out = Command::new(bin())
		.args(["-f", &compose, "-p", &proj, "down"])
		.output()
		.unwrap();
	let gone = pod_json(&proj).is_none();
	assert!(
		up.status.success(),
		"up failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);
	assert!(existed, "the pod must exist before down");
	assert!(
		out.status.success(),
		"down failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	assert!(gone, "down must remove the pod");
}
