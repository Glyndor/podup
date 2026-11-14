//! `x-podman-pod: true` against a real Podman: what each member gets for
//! `/dev/shm` and its hostname.
use std::fs;
use std::process::Command;

use super::pod_fixtures::down;
use super::*;

/// Run `sh -c <script>` in service `svc` of the project and return stdout.
fn exec(compose: &str, proj: &str, svc: &str, script: &str) -> String {
	let out = Command::new(bin())
		.args([
			"-f", compose, "-p", proj, "exec", "-T", svc, "sh", "-c", script,
		])
		.output()
		.unwrap();
	assert!(
		out.status.success(),
		"exec in {svc} failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Podman gives every pod member the pod's `/dev/shm` and ignores the
/// member's own size, so `shm_size` only takes effect on the pod. Two
/// services that disagree both get the larger size; each keeps its own
/// hostname, since the pod shares only the network.
#[tokio::test]
async fn pod_members_get_the_shm_size_and_hostname_they_declare() {
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("pod-shm");
	let yaml = "x-podman-pod: true\nservices:\n  big:\n    image: alpine:latest\n    command: [sleep, \"300\"]\n    shm_size: 128m\n    hostname: big-host\n  small:\n    image: alpine:latest\n    command: [sleep, \"300\"]\n    shm_size: 32m\n    hostname: small-host\n";
	let path = dir.path().join("docker-compose.yml");
	fs::write(&path, yaml).unwrap();
	let compose = path.to_str().unwrap().to_string();
	let up = Command::new(bin())
		.args(["-f", &compose, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();
	assert!(
		up.status.success(),
		"up failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);
	// `df -k` prints the size in KiB: 128m is 131072.
	let script = "hostname; df -k /dev/shm | awk 'NR==2 {print $2}'";
	let big = exec(&compose, &proj, "big", script);
	let small = exec(&compose, &proj, "small", script);
	down(&compose, &proj);
	assert_eq!(big, "big-host\n131072", "big service");
	assert_eq!(small, "small-host\n131072", "small service");
}
