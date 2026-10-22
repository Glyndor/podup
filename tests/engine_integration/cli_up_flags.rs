//! CLI binary integration tests for the `up` flags that change what `up` starts:
//! `--no-build` and `--no-start`.
use super::*;
use std::fs;
use tempfile::tempdir;
#[tokio::test]
async fn cli_up_no_build_skips_building() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-nobuild", std::process::id());
	fs::write(dir.path().join("Dockerfile"), "FROM alpine:latest\n").unwrap();
	let compose = dir.path().join("docker-compose.yml");
	// A build-only service with no prebuilt image: `--no-build` must refuse to
	// build, so `up` fails because there is no image to run.
	fs::write(&compose, "services:\n  app:\n    build: .\n").unwrap();
	let up = run(&[
		"-f",
		compose.to_str().unwrap(),
		"-p",
		&proj,
		"up",
		"-d",
		"--no-build",
	]);
	assert!(
		!up.status.success(),
		"--no-build must not build the image, so up has nothing to run"
	);
	run(&["-f", compose.to_str().unwrap(), "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_up_no_start_creates_without_starting() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-nostart", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let up = run(&["-f", c, "-p", &proj, "up", "--no-start"]);
	assert!(up.status.success(), "up --no-start failed: {:?}", up.stderr);
	assert_eq!(ps_all_count(c, &proj), 1, "container must be created");
	let running = String::from_utf8_lossy(&run(&["-f", c, "-p", &proj, "ps", "-q"]).stdout)
		.lines()
		.filter(|l| !l.trim().is_empty())
		.count();
	assert_eq!(running, 0, "--no-start must not start the container");
	run(&["-f", c, "-p", &proj, "down"]);
}
