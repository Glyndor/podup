//! `install_binary` against a hostile or broken payload, end to end.
//!
//! `install_binary` replaces `std::env::current_exe()`, so calling it from a
//! test would replace the test binary. Instead each case hard-links the test
//! binary into a scratch directory beside it, runs that link as a child that
//! calls `install_binary`, and inspects the link afterwards. The updater only
//! ever renames over the target and never writes into its inode, so the real
//! test binary is untouched either way; what the parent checks is whether the
//! path the child updated still names the original inode.
//!
//! The payloads are what a signed release could still go wrong with: a binary
//! that fails, one cut short, one that reports another version, one that never
//! answers. The signature gate comes before all of them and has its own tests.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use super::install_binary;

/// Environment variable that tells the child which payload to install.
const MODE: &str = "PODUP_HOSTILE_INSTALL_MODE";
/// The version the child asks the self-test to expect.
const EXPECTED: &str = "9.9.9";

fn script(body: &str) -> Vec<u8> {
	format!("#!/bin/sh\n{body}\n").into_bytes()
}

/// The payload for `mode`, and a fragment its failure message must contain.
fn payload(mode: &str) -> (Vec<u8>, &'static str) {
	match mode {
		"exits_nonzero" => (script("exit 1"), "exited with"),
		"truncated_elf" => {
			let exe = std::fs::read(std::env::current_exe().unwrap()).unwrap();
			(exe[..4096].to_vec(), "updated binary")
		}
		"older_release" => (script("echo podup 0.0.1"), "release-metadata tampering"),
		"never_answers" => (script("exec sleep 60"), "did not respond to --version"),
		"orphan_holds_stdout" => (script("sleep 25 &\necho podup 9.9.9"), "unused"),
		"readonly_dir" => (script("exit 1"), "cannot"),
		_ => (script("exit 1"), "exited with"),
	}
}

/// Child half: install the payload named by the environment and check the
/// error. Does nothing when run as an ordinary test.
#[test]
fn child() {
	let Ok(mode) = std::env::var(MODE) else {
		return;
	};
	let (bytes, fragment) = payload(&mode);
	let result = install_binary(&bytes, EXPECTED);
	if mode == "orphan_holds_stdout" {
		return;
	}
	let err = result.expect_err("a payload that fails its self-test must not install");
	let text = err.to_string();
	assert!(text.contains(fragment), "{mode}: {text}");
	if mode != "readonly_dir" {
		assert!(text.contains("rolled back"), "{mode}: {text}");
	}
}

/// A scratch directory next to the test binary (same filesystem, so the hard
/// link works) holding one link to it.
struct Scratch {
	dir: PathBuf,
	link: PathBuf,
}

impl Scratch {
	fn new(mode: &str) -> Self {
		let exe = std::env::current_exe().unwrap();
		let dir = exe
			.parent()
			.unwrap()
			.join(format!("hostile-install-{mode}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let link = dir.join("podup");
		std::fs::hard_link(&exe, &link).unwrap();
		Self { dir, link }
	}

	/// Names in the scratch directory, sorted.
	fn entries(&self) -> Vec<String> {
		let mut names: Vec<String> = std::fs::read_dir(&self.dir)
			.unwrap()
			.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
			.collect();
		names.sort();
		names
	}

	fn inode(&self) -> u64 {
		std::fs::metadata(&self.link).unwrap().ino()
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o755));
		let _ = std::fs::remove_dir_all(&self.dir);
	}
}

/// Run the child for `mode` against `scratch`. Returns whether the child's own
/// assertions passed (with its output when they did not) and how long it took.
fn run_child(scratch: &Scratch, mode: &str) -> (Result<(), String>, Duration) {
	let start = Instant::now();
	let out = Command::new(&scratch.link)
		.args(["--exact", "update::install::hostile_tests::child"])
		.args(["--test-threads=1", "--nocapture"])
		.env(MODE, mode)
		.output()
		.expect("run the linked test binary");
	let result = if out.status.success() {
		Ok(())
	} else {
		Err(format!(
			"{}{}",
			String::from_utf8_lossy(&out.stdout),
			String::from_utf8_lossy(&out.stderr)
		))
	};
	(result, start.elapsed())
}

fn original_inode() -> u64 {
	std::fs::metadata(std::env::current_exe().unwrap())
		.unwrap()
		.ino()
}

/// The path still names the binary it started as, and nothing was left behind.
fn assert_untouched(scratch: &Scratch, mode: &str) {
	assert_eq!(
		scratch.inode(),
		original_inode(),
		"{mode}: the target was replaced and not restored"
	);
	assert_eq!(
		scratch.entries(),
		vec!["podup".to_string()],
		"{mode}: files left beside the binary"
	);
}

fn assert_rolled_back(mode: &str) {
	let scratch = Scratch::new(mode);
	let (child, _) = run_child(&scratch, mode);
	assert!(child.is_ok(), "{mode}: the child failed: {child:?}");
	assert_untouched(&scratch, mode);
}

#[test]
fn a_payload_that_exits_nonzero_is_rolled_back() {
	assert_rolled_back("exits_nonzero");
}

#[test]
fn a_payload_cut_short_is_rolled_back() {
	assert_rolled_back("truncated_elf");
}

#[test]
fn a_payload_that_reports_an_older_release_is_rolled_back() {
	assert_rolled_back("older_release");
}

#[test]
fn a_payload_that_never_answers_is_killed_and_rolled_back() {
	let mode = "never_answers";
	let scratch = Scratch::new(mode);
	let (child, took) = run_child(&scratch, mode);
	assert!(child.is_ok(), "{mode}: the child failed: {child:?}");
	assert!(took < Duration::from_secs(40), "{mode}: took {took:?}");
	assert_untouched(&scratch, mode);
}

#[test]
fn an_install_dir_the_user_cannot_write_leaves_the_binary_alone() {
	let mode = "readonly_dir";
	let scratch = Scratch::new(mode);
	std::fs::set_permissions(&scratch.dir, std::fs::Permissions::from_mode(0o555)).unwrap();
	let (child, _) = run_child(&scratch, mode);
	assert!(child.is_ok(), "{mode}: the child failed: {child:?}");
	assert_untouched(&scratch, mode);
}

/// A payload that exits at once but leaves a background process holding its
/// stdout must not keep `self_test` waiting on the pipe until that process
/// ends: the 10 s bound covers a binary that exits and leaves a descendant
/// behind as well as one that does not exit.
#[test]
fn a_payload_that_leaves_a_background_process_does_not_stall_the_updater() {
	let mode = "orphan_holds_stdout";
	let scratch = Scratch::new(mode);
	let (_, took) = run_child(&scratch, mode);
	assert!(
		took < Duration::from_secs(15),
		"the self-test waited {took:?} on a descendant's pipe"
	);
}
