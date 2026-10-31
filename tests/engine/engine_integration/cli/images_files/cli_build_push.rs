//! CLI binary integration tests for `build` and `push`.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_push_skips_service_without_image() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-pushskip", std::process::id());
	// A build-only service has no `image:` to push, so push is a no-op success.
	fs::write(&compose, "services:\n  app:\n    build: .\n").unwrap();
	let push = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "push"])
		.output()
		.unwrap();
	assert!(
		push.status.success(),
		"push of an imageless service must succeed: {:?}",
		String::from_utf8_lossy(&push.stderr)
	);
}

#[tokio::test]
async fn cli_push_to_unreachable_registry_errors() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-pushfail", std::process::id());
	// Port 1 refuses connections, so the push must fail with a non-zero exit.
	fs::write(
		&compose,
		"services:\n  app:\n    image: localhost:1/podup-nope:1\n",
	)
	.unwrap();
	let push = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"push",
			"--tls-verify",
			"false",
		])
		.output()
		.unwrap();
	// Which failure, not just that one happened. A non-zero exit is satisfied by
	// a missing compose file or an unreachable Podman, neither of which reaches
	// the registry. #1076 is the reason this matters: a failed pull used to be
	// reported as success, and push shares that in-band-error shape: a test that
	// only reads the exit code cannot tell a real registry failure from podup
	// falling over before it tried.
	let err = String::from_utf8_lossy(&push.stderr);
	assert!(
		err.contains("localhost:1/podup-nope"),
		"the failure did not name the image it could not push: {err:?}"
	);
	assert!(
		!push.status.success(),
		"push to an unreachable registry must fail"
	);
}

#[tokio::test]
async fn cli_up_with_build_flag() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-clbld", std::process::id());
	// `down` leaves the built image behind; the guard removes it even when an
	// assertion below panics.
	let _image = super::TestImage::new(format!("{proj}-web"));
	fs::write(dir.path().join("Dockerfile"), "FROM alpine:latest\n").unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    build: .\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let up = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"up",
			"--detach",
			"--build",
		])
		.output()
		.unwrap();
	assert!(
		up.status.success(),
		"up --build failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}

#[tokio::test]
async fn cli_build_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("Dockerfile"), "FROM alpine:latest\n").unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    build: .\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let build = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "build"])
		.output()
		.unwrap();
	assert!(
		build.status.success(),
		"build failed: {}",
		String::from_utf8_lossy(&build.stderr)
	);
}
