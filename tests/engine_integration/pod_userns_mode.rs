//! `x-podman-pod: true` against a real Podman: a `userns_mode` the services declare
//! reaches the pod.
use std::fs;
use std::process::Command;

use super::pod_fixtures::down;
use super::*;
/// A `userns_mode` every service declares is applied to the pod, so a member
/// runs in that user namespace without carrying the key itself.
#[tokio::test]
async fn a_pod_takes_the_services_user_namespace() {
	// Serialize with the rest of the userns lane so two parallel runs do
	// not consume the host's subuid range faster than `down` releases it.
	// `USERNS` is the single shared mutex every test that creates a
	// `--userns=auto` container or pod locks; see the comment at its
	// declaration for the measured pool ceiling. Held before the podman
	// availability check, the same way `userns_pod.rs` does it, so a
	// skipped run cannot race an allocation it never started.
	let _guard = USERNS.lock().await;
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("pod-userns");
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    userns_mode: auto\n  db:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    userns_mode: auto\n";
	let path = dir.path().join("docker-compose.yml");
	fs::write(&path, yaml).unwrap();
	let compose = path.to_str().unwrap().to_string();
	let up = Command::new(bin())
		.args(["-f", &compose, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();
	let in_pod = Command::new("podman")
		.args([
			"exec",
			&format!("{proj}-web-1"),
			"cat",
			"/proc/self/uid_map",
		])
		.output()
		.unwrap();
	let plain = Command::new("podman")
		.args(["run", "--rm", "alpine:latest", "cat", "/proc/self/uid_map"])
		.output()
		.unwrap();
	down(&compose, &proj);
	assert!(
		up.status.success(),
		"up failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);
	let in_pod = String::from_utf8_lossy(&in_pod.stdout).to_string();
	let plain = String::from_utf8_lossy(&plain.stdout).to_string();
	assert_ne!(
		in_pod.trim(),
		plain.trim(),
		"a member of a pod created with userns auto must not run in the default rootless mapping"
	);
}
