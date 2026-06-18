//! Niche-command CLI integration tests (wait/export/commit), split for the
//! source line limit.
use std::fs;
use std::process::Command;
use tempfile::tempdir;

use super::*;

fn run(args: &[&str]) -> std::process::Output {
	Command::new(bin()).args(args).output().unwrap()
}
#[tokio::test]
async fn cli_wait_prints_exit_code() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-wait", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  job:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"exit 0\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	run(&["-f", c, "-p", &proj, "up", "-d"]);
	let out = run(&["-f", c, "-p", &proj, "wait", "job"]);
	assert!(out.status.success(), "wait failed: {:?}", out.stderr);
	assert!(
		String::from_utf8_lossy(&out.stdout)
			.lines()
			.any(|l| l.trim() == "0"),
		"wait must print the exit code 0"
	);
	run(&["-f", c, "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_export_writes_tar() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-export", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();
	let tar = dir.path().join("rootfs.tar");

	run(&["-f", c, "-p", &proj, "up", "-d"]);
	let out = run(&[
		"-f",
		c,
		"-p",
		&proj,
		"export",
		"-o",
		tar.to_str().unwrap(),
		"web",
	]);
	run(&["-f", c, "-p", &proj, "down"]);
	assert!(out.status.success(), "export failed: {:?}", out.stderr);
	let meta = fs::metadata(&tar).expect("export must create the tar file");
	assert!(meta.len() > 0, "exported tar must be non-empty");
}

#[tokio::test]
async fn cli_commit_creates_image() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-commit", std::process::id());
	let img = format!("podup-commit-test-{}:latest", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	run(&["-f", c, "-p", &proj, "up", "-d"]);
	let out = run(&["-f", c, "-p", &proj, "commit", "web", &img]);
	run(&["-f", c, "-p", &proj, "down"]);
	let exists = std::process::Command::new("podman")
		.args(["image", "exists", &img])
		.status()
		.map(|s| s.success())
		.unwrap_or(false);
	// Clean up the committed image regardless.
	let _ = std::process::Command::new("podman")
		.args(["rmi", "-f", &img])
		.output();
	assert!(out.status.success(), "commit failed: {:?}", out.stderr);
	assert!(exists, "commit must create the target image");
}
