//! CLI binary integration tests for `pull`.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_pull_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(&compose, "services:\n  web:\n    image: alpine:latest\n").unwrap();

	let pull = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "pull"])
		.output()
		.unwrap();
	assert!(pull.status.success());
	// Also stderr, measured. #1076 is the reason the reported outcome matters
	// here: a failed pull used to arrive as an in-band error line on a 200 and be
	// dropped, so `pull` exited 0 having pulled nothing.
	let err = String::from_utf8_lossy(&pull.stderr);
	assert!(
		err.contains("alpine:latest") && err.contains("Pulled"),
		"pull did not report pulling the image: {err:?}"
	);
}
