//! CLI binary integration tests for the `--pull` policy, on `up` and on `pull`.
use super::*;
use std::fs;
use tempfile::tempdir;
#[tokio::test]
async fn cli_up_pull_never_starts_present_image() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-pullnever", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();
	// Ensure the image is present, then `--pull never` must still start it.
	run_ok(&["-f", c, "-p", &proj, "up", "-d"]);
	run(&["-f", c, "-p", &proj, "down"]);
	let up = run(&["-f", c, "-p", &proj, "up", "-d", "--pull", "never"]);
	assert!(
		up.status.success(),
		"up --pull never failed: {:?}",
		up.stderr
	);
	run(&["-f", c, "-p", &proj, "down"]);
}

#[tokio::test]
async fn cli_pull_quiet_succeeds() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-pullq", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(&compose, "services:\n  web:\n    image: alpine:latest\n").unwrap();
	let pull = run(&["-f", compose.to_str().unwrap(), "-p", &proj, "pull", "-q"]);
	assert!(
		pull.status.success(),
		"pull -q failed: {}",
		String::from_utf8_lossy(&pull.stderr)
	);
}
