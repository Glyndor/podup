//! The interface name a service asks for on a network reaches the pod's
//! members, against a real Podman: the pod is created with the attachment
//! `interface_name`, and a member's namespace has that interface.

use std::process::Command;

use super::pod_fixtures::down;
use super::*;

#[tokio::test]
async fn the_interface_name_of_a_pod_network_is_applied() {
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("pod-ifname");
	let compose = dir.path().join("docker-compose.yml");
	std::fs::write(
		&compose,
		"x-podman-pod: true\nservices:\n  web:\n    image: busybox:latest\n    command: [sleep, '300']\n    networks:\n      n:\n        interface_name: eth7\nnetworks:\n  n: {}\n",
	)
	.unwrap();
	let compose = compose.to_str().unwrap().to_string();
	let up = Command::new(bin())
		.args(["-f", &compose, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();
	let links = Command::new("podman")
		.args(["exec", &format!("{proj}-web-1"), "ip", "-o", "link"])
		.output()
		.unwrap();
	down(&compose, &proj);
	assert!(
		up.status.success(),
		"up failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);
	let links = String::from_utf8_lossy(&links.stdout).to_string();
	assert!(links.contains("eth7"), "no eth7 in the pod: {links}");
}
