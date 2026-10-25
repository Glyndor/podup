//! The refusal of a compose file that declares a top-level `jobs:` section,
//! seen from the command line.
//!
//! podup runs no job runner, so a file with jobs is refused instead of
//! starting a project whose jobs would never run. Every command reads the
//! compose file through the same loader, so the refusal has to show up in
//! `config` (the command that shows what would run) and in `up` (the command
//! that would start it), and in `up` it has to happen before Podman is
//! contacted: a job that is refused late has already created containers.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

/// The refusal every entry point carries.
const REFUSAL: &str = "jobs: the top-level jobs section is not supported by podup yet; \
	remove it or run those containers with another tool";

/// A socket path that cannot be connected to, so a command that reaches the
/// engine fails with a connection error instead of doing anything.
const DEAD_SOCKET: &str = "unix:///nonexistent/podup-jobs-section-test.sock";

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Write `contents` as a compose file in a temp dir of its own.
///
/// The directory is unique per call, not per process: cargo runs the tests of
/// one binary as threads, so a path keyed on the pid would be shared by every
/// test in this file and two tests writing the same file would race.
fn compose_file(name: &str, contents: &str) -> (TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("create temp dir");
	let path = dir.path().join(name);
	fs::write(&path, contents).expect("write compose file");
	(dir, path)
}

/// A compose file with one job.
fn compose_with_a_job() -> (TempDir, PathBuf) {
	compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: nginx\njobs:\n  backup:\n    image: alpine\n    \
		 triggers:\n      - type: schedule\n        schedule: daily\n",
	)
}

#[test]
fn config_refuses_a_file_with_a_job() {
	let (_dir, file) = compose_with_a_job();
	let output = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "config"])
		.output()
		.expect("run podup config");

	assert!(
		!output.status.success(),
		"config must fail on a file with jobs; stdout:\n{}",
		String::from_utf8_lossy(&output.stdout)
	);
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains(REFUSAL),
		"the refusal must say what is wrong; got stderr:\n{stderr}"
	);
	assert!(
		stderr.contains("backup"),
		"the refusal must name the job to remove; got stderr:\n{stderr}"
	);
}

#[test]
fn up_refuses_a_file_with_a_job_before_contacting_podman() {
	let (_dir, file) = compose_with_a_job();
	let output = Command::new(bin())
		.args([
			"-f",
			file.to_str().unwrap(),
			"--socket",
			DEAD_SOCKET,
			"up",
			"-d",
		])
		.output()
		.expect("run podup up");

	assert!(
		!output.status.success(),
		"up must fail on a file with jobs; stdout:\n{}",
		String::from_utf8_lossy(&output.stdout)
	);
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains(REFUSAL),
		"the refusal must say what is wrong; got stderr:\n{stderr}"
	);
	// The socket cannot be connected to, so a command that reached the engine
	// would fail with a connection error instead. The refusal came first, so
	// nothing was created and nothing was asked of Podman.
	assert!(
		!stderr.contains("socket connection error"),
		"the refusal must come before Podman is contacted; got stderr:\n{stderr}"
	);
}

#[test]
fn config_accepts_an_empty_jobs_map() {
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: nginx\njobs: {}\n",
	);
	let output = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "config"])
		.output()
		.expect("run podup config");

	assert!(
		output.status.success(),
		"an empty jobs map declares nothing; stderr:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(
		stdout.contains("services:") && !stdout.contains("jobs:"),
		"the resolved file carries no jobs section; got stdout:\n{stdout}"
	);
}

#[test]
fn config_accepts_a_null_jobs_value() {
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: nginx\njobs:\n",
	);
	let output = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "config"])
		.output()
		.expect("run podup config");

	assert!(
		output.status.success(),
		"a null jobs value declares nothing; stderr:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(
		stdout.contains("services:") && !stdout.contains("jobs:"),
		"the resolved file carries no jobs section; got stdout:\n{stdout}"
	);
}

#[test]
fn config_is_unchanged_for_a_file_without_jobs() {
	let (_dir, file) = compose_file("compose.yaml", "services:\n  web:\n    image: nginx\n");
	let output = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "config"])
		.output()
		.expect("run podup config");

	assert!(
		output.status.success(),
		"a file without jobs parses; stderr:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(
		stdout.contains("services:") && !stdout.contains("jobs:"),
		"the resolved file carries no jobs section; got stdout:\n{stdout}"
	);
}
