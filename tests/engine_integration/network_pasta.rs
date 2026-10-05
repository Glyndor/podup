//! `network_mode: "pasta:<options>"` against a live Podman (#1994).
//!
//! The mode and its options must reach libpod apart, as `podman run --network
//! pasta:...` sends them. Sent as one string, Podman refused the create with
//! `invalid network "pasta:-m,1400"`. The option is checked by its effect: pasta
//! sets the container interface's MTU to 1400 instead of its default 65520, so
//! a mode that arrived without its options fails the MTU assertion.
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
		"services:\n  web:\n    image: docker.io/library/alpine:3.20\n    command: [\"sleep\", \"300\"]\n    network_mode: \"pasta:-m,1400\"\n",
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
		"up with network_mode pasta:-m,1400 failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);
	let mtus = Command::new("podman")
		.args([
			"exec",
			&format!("{project}-web-1"),
			"sh",
			"-c",
			"for i in /sys/class/net/*; do [ \"${i##*/}\" = lo ] || cat \"$i/mtu\"; done",
		])
		.output()
		.expect("run podman exec");
	let mtus = String::from_utf8_lossy(&mtus.stdout);
	assert_eq!(
		mtus.trim(),
		"1400",
		"pasta's -m 1400 should set the container interface MTU; got {mtus:?}"
	);
}
