//! #1893: a compose healthcheck that sets only timings keeps the image's probe.
//!
//! The request carries no `test`, and libpod merges field by field and fills the
//! probe from the image (measured on Podman 6.2.0). This runs the real thing: it
//! builds an image with a `HEALTHCHECK`, brings the service up with only an
//! `interval` and reads the healthcheck the container ended up with.

use super::{bin, podman, podman_socket_url, proj};
use std::process::Command;

#[tokio::test]
async fn timings_without_a_test_keep_the_probe_of_the_image() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let project = proj("hcinherit");
	std::fs::write(
		dir.path().join("Dockerfile"),
		"FROM alpine:3.20\nHEALTHCHECK --interval=7s --timeout=4s --retries=5 CMD echo from-image\nCMD [\"sleep\",\"3600\"]\n",
	)
	.unwrap();
	let compose = dir.path().join("docker-compose.yml");
	std::fs::write(
		&compose,
		"services:\n  app:\n    build: .\n    image: proj/hcinherit:1\n    stop_grace_period: 0s\n    healthcheck:\n      interval: 3s\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let up = Command::new(bin())
		.args(["-f", c, "-p", &project, "up", "-d", "--build"])
		.env("PODMAN_SOCKET", &socket)
		.output()
		.unwrap();
	let probe = Command::new("podman")
		.args([
			"--url",
			&socket,
			"inspect",
			"--format",
			"{{json .Config.Healthcheck}}",
			&format!("{project}-app-1"),
		])
		.output()
		.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
		.unwrap_or_default();
	let _ = Command::new(bin())
		.args(["-f", c, "-p", &project, "down", "--rmi", "local"])
		.env("PODMAN_SOCKET", &socket)
		.output();

	assert!(
		up.status.success(),
		"up failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);
	assert!(
		probe.contains("from-image"),
		"the image probe is lost: {probe}"
	);
	assert!(
		probe.contains("3000000000"),
		"the file's interval is lost: {probe}"
	);
	assert!(
		probe.contains("4000000000"),
		"the image's timeout is lost: {probe}"
	);
}
