//! Scale integration tests (`up --scale` and the `scale` subcommand) against a
//! real Podman daemon. Skip gracefully when Podman is unreachable.
use std::fs;
use std::process::Command;
use tempfile::tempdir;

use super::*;

/// Number of running containers in the project (counts `ps -q` output lines).
fn running_count(compose: &str, proj: &str) -> usize {
	let out = Command::new(bin())
		.args(["-f", compose, "-p", proj, "ps", "-q"])
		.output()
		.unwrap();
	String::from_utf8_lossy(&out.stdout)
		.lines()
		.filter(|l| !l.trim().is_empty())
		.count()
}

fn run(args: &[&str]) -> std::process::Output {
	Command::new(bin()).args(args).output().unwrap()
}

#[tokio::test]
async fn up_scale_creates_replicas_and_down_removes_them() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-upscale", std::process::id());
	fs::write(
		&compose,
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let up = run(&[
		"-f", c, "-p", &proj, "up", "--detach", "--scale", "worker=3",
	]);
	assert!(up.status.success(), "up --scale failed: {:?}", up.stderr);
	assert_eq!(
		running_count(c, &proj),
		3,
		"expected 3 replicas after up --scale"
	);

	let down = run(&["-f", c, "-p", &proj, "down"]);
	assert!(down.status.success(), "down failed: {:?}", down.stderr);
	assert_eq!(
		running_count(c, &proj),
		0,
		"down must remove scaled replicas"
	);
}

#[tokio::test]
async fn scale_subcommand_scales_up_then_down() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-scalecmd", std::process::id());
	fs::write(
		&compose,
		"services:\n  worker:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	run(&["-f", c, "-p", &proj, "up", "--detach"]);
	assert_eq!(running_count(c, &proj), 1);

	let up = run(&["-f", c, "-p", &proj, "scale", "worker=3"]);
	assert!(up.status.success(), "scale up failed: {:?}", up.stderr);
	assert_eq!(running_count(c, &proj), 3, "scale up must add replicas");

	let down = run(&["-f", c, "-p", &proj, "scale", "worker=1"]);
	assert!(
		down.status.success(),
		"scale down failed: {:?}",
		down.stderr
	);
	assert_eq!(running_count(c, &proj), 1, "scale down must remove surplus");

	run(&["-f", c, "-p", &proj, "down"]);
	assert_eq!(running_count(c, &proj), 0);
}

#[tokio::test]
async fn up_scale_fixed_host_port_fails_fast() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-scaleport", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    ports:\n      - \"8085:80\"\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let out = run(&["-f", c, "-p", &proj, "up", "--detach", "--scale", "web=2"]);
	assert!(!out.status.success(), "scaling a fixed host port must fail");
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains("only one container can bind a host port"),
		"expected port-conflict guidance, got: {stderr}"
	);
	// Clean up anything the failed attempt may have created.
	run(&["-f", c, "-p", &proj, "down"]);
}
