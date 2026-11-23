//! `down --rmi all` leaves an image another container still uses, and exits
//! zero: Podman answers that removal with a 409 conflict, which podup reads as
//! "in use". A 500 is no longer read that way, so this is the live check that
//! the answer it relies on is the one Podman gives.

use std::fs;
use std::process::Command;

use super::*;

fn podman_cli(args: &[&str]) -> std::process::Output {
	Command::new("podman")
		.args(args)
		.output()
		.expect("run the podman CLI")
}

/// Removes, by exact name, what the test made outside the project.
struct Cleanup {
	names: Vec<String>,
	image: String,
	compose: String,
	project: String,
}

impl Drop for Cleanup {
	fn drop(&mut self) {
		let _ = run(&["-f", &self.compose, "-p", &self.project, "down"]);
		for name in &self.names {
			let _ = podman_cli(&["rm", "--force", "--time", "0", name]);
		}
		let _ = podman_cli(&["rmi", "--force", &self.image]);
	}
}

#[tokio::test]
async fn down_rmi_leaves_an_image_that_another_container_uses() {
	if super::podman().await.is_none() {
		return;
	}
	let pid = std::process::id();
	let image = format!("localhost/podup-rmi-{pid}:1");
	let source = format!("podup-rmi-{pid}-source");
	let other = format!("podup-rmi-{pid}-other");
	let project = format!("t{pid}-rmi");
	let dir = tempfile::tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let compose_path = compose.to_str().unwrap().to_string();
	let _cleanup = Cleanup {
		names: vec![source.clone(), other.clone()],
		image: image.clone(),
		compose: compose_path.clone(),
		project: project.clone(),
	};

	// A committed image has one name and one id, so removing it is a real
	// removal and not an untag.
	assert!(
		podman_cli(&["pull", "--quiet", "docker.io/library/busybox:latest"])
			.status
			.success(),
		"could not pull busybox"
	);
	for args in [
		vec![
			"create",
			"--name",
			&source,
			"docker.io/library/busybox:latest",
			"true",
		],
		vec!["commit", "--quiet", &source, &image],
		vec!["create", "--name", &other, &image, "true"],
	] {
		let out = podman_cli(&args);
		assert!(out.status.success(), "{args:?}: {out:?}");
	}
	fs::write(
		&compose,
		format!(
			"services:\n  web:\n    image: {image}\n    pull_policy: never\n    command: [\"sleep\", \"300\"]\n"
		),
	)
	.unwrap();

	let up = run(&["-f", &compose_path, "-p", &project, "up", "-d"]);
	assert!(
		up.status.success(),
		"{}",
		String::from_utf8_lossy(&up.stderr)
	);
	let down = run(&["-f", &compose_path, "-p", &project, "down", "--rmi", "all"]);
	assert!(
		down.status.success(),
		"an image another container uses must be skipped, not fail the command: {}",
		String::from_utf8_lossy(&down.stderr)
	);
	assert!(
		podman_cli(&["image", "exists", &image]).status.success(),
		"the image another container uses was removed"
	);
}
