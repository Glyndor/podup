//! `x-podman-pod: true` against a real Podman: services inside the pod resolve each
//! other to localhost.
use std::process::Command;

use super::pod_fixtures::{down, write_compose};
use super::*;
#[tokio::test]
async fn services_in_a_pod_resolve_each_other_to_localhost() {
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("pod-dns");
	let compose = write_compose(dir.path(), "");
	let up = Command::new(bin())
		.args(["-f", &compose, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();
	let resolved = Command::new("podman")
		.args(["exec", &format!("{proj}-web-1"), "getent", "hosts", "db"])
		.output()
		.unwrap();
	let reached = Command::new("podman")
		.args([
			"exec",
			&format!("{proj}-web-1"),
			"nc",
			"-z",
			"-w",
			"3",
			"db",
			"5432",
		])
		.output()
		.unwrap();
	down(&compose, &proj);
	assert!(
		up.status.success(),
		"up failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);
	let hosts = String::from_utf8_lossy(&resolved.stdout).to_string();
	assert!(
		hosts.starts_with("127.0.0.1"),
		"db must resolve to the shared namespace, got: {hosts:?}"
	);
	assert!(
		reached.status.success(),
		"a TCP connect to db:5432 by name must succeed inside the pod: {}",
		String::from_utf8_lossy(&reached.stderr)
	);
}
