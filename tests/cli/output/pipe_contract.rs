//! What the binary does when the reader of its output is gone, and what it
//! does when the output cannot be written at all.
//!
//! `podup ps | head -1` closes the pipe while the command is still writing.
//! Rust ignores SIGPIPE, so a direct `println!` panics on that write, and
//! the release profile builds with `panic = "abort"`, which turns the panic
//! into SIGABRT (exit 134) and a bug-report notice about a crash nobody has.
//!
//! The contract: the command ends quietly, with the success status, and says
//! nothing. A reader that went away is not something the operator can act on,
//! and a report about it would describe a bug that is not there.
//!
//! The other side of the same contract is the write that fails for a reason
//! the operator can act on: a full disk. There the command has to fail and
//! say so, because a table that lost its last row and exited zero is a lie a
//! script cannot detect.
//!
//! The reader is closed before the command writes, so the write finds no
//! reader and fails with a broken pipe on every run.

mod harness;

use std::fs;
use std::process::{Command, Stdio};
use tempfile::tempdir;

use harness::{podman_up, Project};

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Run `podup <args>` with its stdout closed before it writes, and return what
/// the process said and how it ended.
fn run_with_closed_stdout(args: &[&str]) -> (Option<i32>, String) {
	let mut child = Command::new(bin())
		.args(args)
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.expect("spawn podup");
	// Close the read end while the command is still starting: the write that
	// follows finds no reader. The close happens in the parent right after
	// the spawn, and the command needs to load, parse its arguments and
	// render its output, which takes far longer than that.
	drop(child.stdout.take());
	let out = child.wait_with_output().expect("podup finished");
	(
		out.status.code(),
		String::from_utf8_lossy(&out.stderr).into_owned(),
	)
}

/// The device that accepts every write and fails every one of them.
///
/// Linux has it; other systems do not, and a test that needs it says so
/// rather than pretending to have measured something.
#[cfg(target_os = "linux")]
fn full_device() -> Option<std::fs::File> {
	fs::OpenOptions::new().write(true).open("/dev/full").ok()
}

#[cfg(not(target_os = "linux"))]
fn full_device() -> Option<std::fs::File> {
	None
}

/// Run `podup <args>` with its stdout on the full device, and return what the
/// process said and how it ended.
fn run_with_full_stdout(args: &[&str], full: std::fs::File) -> (Option<i32>, String) {
	let child = Command::new(bin())
		.args(args)
		.stdout(Stdio::from(full))
		.stderr(Stdio::piped())
		.spawn()
		.expect("spawn podup");
	let out = child.wait_with_output().expect("podup finished");
	(
		out.status.code(),
		String::from_utf8_lossy(&out.stderr).into_owned(),
	)
}

/// A compose file with one service, for the commands that read the model and
/// write to stdout without asking Podman for anything.
fn compose_file() -> (tempfile::TempDir, String) {
	let dir = tempdir().expect("temp dir");
	let path = dir.path().join("compose.yaml");
	fs::write(
		&path,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"600\"]\n",
	)
	.expect("write compose file");
	let path = path.to_string_lossy().into_owned();
	(dir, path)
}

#[test]
fn a_closed_stdout_ends_the_command_quietly() {
	// `help` writes through the writer in `podup::ui`, the one place that
	// decides what a closed pipe means.
	let (code, stderr) = run_with_closed_stdout(&["help"]);
	assert_eq!(
		code,
		Some(0),
		"a closed pipe must not end the command with a crash; the status was {code:?}"
	);
	assert!(
		stderr.is_empty(),
		"a closed pipe must say nothing; stderr was: {stderr}"
	);
}

#[test]
fn a_closed_stdout_ends_a_long_output_quietly() {
	// The completion script is longer than a pipe buffer, so this case is
	// the one a reader that goes away mid-write produces.
	let (code, stderr) = run_with_closed_stdout(&["completions", "bash"]);
	assert_eq!(
		code,
		Some(0),
		"a closed pipe must not end the command with a crash; the status was {code:?}"
	);
	assert!(
		stderr.is_empty(),
		"a closed pipe must say nothing; stderr was: {stderr}"
	);
}

/// The same contract for a command that renders a document.
///
/// `config` wrote straight to stdout and turned the failed write into a
/// command failure, so `podup config | head -1` ended with an error about a
/// broken pipe: a reader that closed the pipe is not a failure, and the
/// message described a problem the operator cannot act on.
#[test]
fn a_closed_stdout_ends_the_config_command_quietly() {
	let (_dir, compose) = compose_file();
	let (code, stderr) = run_with_closed_stdout(&["-f", &compose, "config"]);
	assert_eq!(
		code,
		Some(0),
		"a closed pipe must not end the command with a crash; the status was {code:?}"
	);
	assert!(
		stderr.is_empty(),
		"a closed pipe must say nothing; stderr was: {stderr}"
	);
}

/// A write that fails for a reason the operator can act on has to end the
/// command with a failure and say what happened.
///
/// The command that renders the labelled status lines wrote them to stdout
/// and dropped the failure, so a full disk left the answer half-written and
/// the command still exited zero.
#[test]
fn a_full_device_makes_the_status_command_fail() {
	let Some(full) = full_device() else {
		return;
	};
	let (_dir, compose) = compose_file();
	let (code, stderr) = run_with_full_stdout(&["-f", &compose, "autostart", "status"], full);
	assert_ne!(
		code,
		Some(0),
		"a write that failed must not end the command with success"
	);
	assert!(
		stderr.contains("error"),
		"the failure must be reported; stderr was: {stderr}"
	);
}

/// The same for the rendered document.
#[test]
fn a_full_device_makes_the_config_command_fail() {
	let Some(full) = full_device() else {
		return;
	};
	let (_dir, compose) = compose_file();
	let (code, stderr) = run_with_full_stdout(&["-f", &compose, "config"], full);
	assert_ne!(
		code,
		Some(0),
		"a write that failed must not end the command with success"
	);
	assert!(
		stderr.contains("error"),
		"the failure must be reported; stderr was: {stderr}"
	);
}

/// The table is the case that lost rows.
///
/// `Table::print` dropped the write error, so the listing ended with a
/// success status and fewer rows than there were: a script reading it saw a
/// project with no containers where the truth was a disk with no room.
#[tokio::test]
async fn a_full_device_makes_the_container_listing_fail() {
	let Some(full) = full_device() else {
		return;
	};
	if !podman_up().await {
		return;
	}
	let p = Project::start("full");
	let (code, stderr) = run_with_full_stdout(&["-f", &p.compose, "-p", &p.name, "ps"], full);
	assert_ne!(
		code,
		Some(0),
		"a write that failed must not end the command with success"
	);
	assert!(
		stderr.contains("error"),
		"the failure must be reported; stderr was: {stderr}"
	);
}

/// The image listing is the same table through another command.
#[tokio::test]
async fn a_full_device_makes_the_image_listing_fail() {
	let Some(full) = full_device() else {
		return;
	};
	if !podman_up().await {
		return;
	}
	let p = Project::start("fullimg");
	let (code, stderr) = run_with_full_stdout(&["-f", &p.compose, "-p", &p.name, "images"], full);
	assert_ne!(
		code,
		Some(0),
		"a write that failed must not end the command with success"
	);
	assert!(
		stderr.contains("error"),
		"the failure must be reported; stderr was: {stderr}"
	);
}

/// A reader that closes the pipe is not a failure, whichever command it was.
#[tokio::test]
async fn a_closed_stdout_ends_the_container_listing_quietly() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("closed");
	let (code, stderr) = run_with_closed_stdout(&["-f", &p.compose, "-p", &p.name, "ps"]);
	assert_eq!(
		code,
		Some(0),
		"a closed pipe must not end the command with a crash; the status was {code:?}"
	);
	assert!(
		stderr.is_empty(),
		"a closed pipe must say nothing; stderr was: {stderr}"
	);
}
