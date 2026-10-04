//! `network_mode: "pasta:<options>"` against a live Podman (#1994).
//!
//! The mode and its options must reach libpod apart, as `podman run --network
//! pasta:...` sends them. Sent as one string, Podman refused the create with
//! `invalid network "pasta:-4"`.
use std::fs;
use std::process::Command;

use super::*;

#[tokio::test]
async fn up_accepts_pasta_with_options() {
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().expect("tempdir");
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: docker.io/library/alpine:3.20\n    command: [\"sleep\", \"300\"]\n    network_mode: \"pasta:-4\"\n",
	)
	.expect("write compose");
	let project = proj("pasta");
	let run = |args: &[&str]| {
		Command::new(bin())
			.args(["-f", compose.to_str().unwrap(), "-p", &project])
			.args(args)
			.output()
			.expect("run podup")
	};
	struct Down<F: Fn()>(F);
	impl<F: Fn()> Drop for Down<F> {
		fn drop(&mut self) {
			(self.0)();
		}
	}
	let _down = Down(|| {
		let _ = run(&["down", "-t", "0"]);
	});

	let up = run(&["up", "-d"]);
	assert!(
		up.status.success(),
		"up with network_mode pasta:-4 failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);
	let inspect = Command::new("podman")
		.args([
			"inspect",
			&format!("{project}-web-1"),
			"--format",
			"{{.State.Running}} {{.HostConfig.NetworkMode}}",
		])
		.output()
		.expect("run podman inspect");
	let state = String::from_utf8_lossy(&inspect.stdout);
	assert!(
		state.trim().starts_with("true pasta"),
		"container should run on pasta: {state:?}"
	);
}
