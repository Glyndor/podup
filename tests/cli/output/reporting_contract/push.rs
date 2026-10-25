//! `push` reports the image it is pushing, and `--quiet` suppresses that line.
//!
//! Split out from `tests/cli/output/reporting_contract.rs`; the shared fixtures are
//! declared in the crate root, which the scenario files inherit via
//! `use super::*`.

use std::fs;
use std::process::Command;
use tempfile::tempdir;

use super::*;
use harness::{bin, podman_up};

/// `push` says something. It used to say nothing at all.
///
/// Its only two user-facing lines were `tracing::info!`, and the CLI floors
/// tracing at `warn`, so a push that genuinely uploaded the image wrote zero
/// bytes to stdout and stderr and exited 0, measured against a real local
/// registry, which afterwards listed the repository. This asserts the line is
/// back and that it goes to stderr, leaving stdout a clean pipe.
///
/// Deliberately pointed at a registry that cannot exist (port 1) with an image
/// that is not present locally: the progress line is emitted before the API
/// call, so the assertion needs no registry and no build, and the command still
/// fails afterwards, which is also what pins the line as *progress* rather
/// than a success message.
#[tokio::test]
async fn push_reports_the_image_it_is_pushing() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		"services:\n  x:\n    image: localhost:1/absent:1\n",
	)
	.unwrap();
	let out = Command::new(bin())
		.args([
			"-f",
			&compose.to_string_lossy(),
			"push",
			"--tls-verify=false",
		])
		.output()
		.expect("run podup push");
	let stderr = String::from_utf8_lossy(&out.stderr);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		stderr.contains("localhost:1/absent:1") && stderr.contains("Pushing"),
		"push must report the image on stderr; got stderr:\n{stderr}"
	);
	assert!(
		stdout.trim().is_empty(),
		"push must leave stdout a clean pipe; got stdout:\n{stdout}"
	);
}

/// `push --quiet` suppresses the progress lines. The flag existed while there
/// was no output for it to suppress, so nothing had ever exercised it.
#[tokio::test]
async fn push_quiet_suppresses_the_progress_lines() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		"services:\n  x:\n    image: localhost:1/absent:1\n",
	)
	.unwrap();
	let out = Command::new(bin())
		.args([
			"-f",
			&compose.to_string_lossy(),
			"push",
			"--quiet",
			"--tls-verify=false",
		])
		.output()
		.expect("run podup push --quiet");
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!stderr.contains("Pushing"),
		"--quiet must suppress the progress line; got stderr:\n{stderr}"
	);
}
