//! Exercise verified replacement on disposable executables without release fetches.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

#[path = "../support/fake_engine.rs"]
mod fake_engine;
#[path = "../support/host_files.rs"]
mod host_files;

use host_files::{Running, BUDGET};

const TARGET: &str = "PODUP_H2_TARGET";
const PAYLOAD: &str = "PODUP_H2_PAYLOAD";
const GOOD: &[u8] = b"#!/bin/sh\necho podup 9.9.9\n";
const OLD: &[u8] = b"#!/bin/sh\necho podup 1.0.0\n";

#[test]
fn child_install() {
	let Some(target) = std::env::var_os(TARGET) else {
		return;
	};
	let payload = fs::read(std::env::var_os(PAYLOAD).unwrap()).unwrap();
	match podup::update::test_install_verified_at(Path::new(&target), &payload, "9.9.9") {
		Ok(()) => println!("INSTALL_OK"),
		Err(error) => println!("INSTALL_ERR: {error}"),
	}
}

fn start(target: &Path, payload: &Path) -> Running {
	let mut command = Command::new(std::env::current_exe().unwrap());
	command
		.args([
			"--exact",
			"child_install",
			"--nocapture",
			"--test-threads=1",
		])
		.env(TARGET, target)
		.env(PAYLOAD, payload)
		.env("PODUP_H2_READY", target.with_extension("ready"))
		.env("PODUP_H2_RELEASE", target.with_extension("release"));
	Running::command(command)
}

fn finish(child: &mut Running, diagnostic: &str) {
	let status = child.wait();
	let text = child.text();
	assert!(status.success(), "child panic or signal: {status}: {text}");
	assert!(text.contains(diagnostic), "missing {diagnostic:?}: {text}");
}

fn target(root: &Path, mode: u32) -> std::path::PathBuf {
	let target = root.join("podup");
	fs::write(&target, OLD).unwrap();
	fs::set_permissions(&target, fs::Permissions::from_mode(mode)).unwrap();
	target
}

fn no_staging_files(root: &Path) {
	assert!(!root.join("podup.old").exists(), "backup not cleaned up");
	for entry in fs::read_dir(root).unwrap() {
		assert!(
			!entry
				.unwrap()
				.file_name()
				.to_string_lossy()
				.contains(".update-"),
			"staging file leaked"
		);
	}
}

#[test]
fn empty_payload_is_rolled_back_without_losing_the_original_inode() {
	let dir = tempfile::tempdir().unwrap();
	let target = target(dir.path(), 0o755);
	let inode = fs::metadata(&target).unwrap().ino();
	let payload = dir.path().join("payload");
	fs::write(&payload, []).unwrap();
	let mut child = start(&target, &payload);
	finish(&mut child, "rolled back");
	assert!(
		child.text().contains("could not run the updated binary"),
		"{}",
		child.text()
	);
	assert_eq!(fs::read(&target).unwrap(), OLD);
	assert_eq!(fs::metadata(&target).unwrap().ino(), inode);
	assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o755);
	no_staging_files(dir.path());
}

#[test]
fn a_readonly_executable_can_be_replaced_without_writing_its_inode() {
	let dir = tempfile::tempdir().unwrap();
	let target = target(dir.path(), 0o555);
	let original = dir.path().join("original-inode");
	fs::hard_link(&target, &original).unwrap();
	let payload = dir.path().join("payload");
	fs::write(&payload, GOOD).unwrap();
	let mut child = start(&target, &payload);
	finish(&mut child, "INSTALL_OK");
	assert_eq!(fs::read(&target).unwrap(), GOOD);
	assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o555);
	assert_eq!(fs::read(&original).unwrap(), OLD);
	assert_eq!(fs::metadata(&original).unwrap().mode() & 0o777, 0o555);
	assert_ne!(
		fs::metadata(&target).unwrap().ino(),
		fs::metadata(&original).unwrap().ino()
	);
	no_staging_files(dir.path());
}

#[test]
fn concurrent_processes_cannot_share_the_backup_during_self_test() {
	let dir = tempfile::tempdir().unwrap();
	let target = target(dir.path(), 0o755);
	let payload = dir.path().join("blocking-payload");
	// Signal from inside the self-test while the transaction owns its backup.
	// Bound the shell loop too, so killing an updater cannot orphan a waiter.
	fs::write(&payload, b"#!/bin/sh\n: > \"$PODUP_H2_READY\"\ni=0\nwhile [ ! -f \"$PODUP_H2_RELEASE\" ]; do\n i=$((i+1))\n [ \"$i\" -lt 300 ] || exit 1\n sleep 0.01\ndone\necho podup 9.9.9\n").unwrap();
	let mut first = start(&target, &payload);
	let deadline = Instant::now() + BUDGET;
	while !target.with_extension("ready").exists() {
		assert!(
			Instant::now() < deadline,
			"first update did not reach self-test: {}",
			first.text()
		);
		std::thread::sleep(std::time::Duration::from_millis(10));
	}
	assert_eq!(fs::read(target.with_extension("old")).unwrap(), OLD);
	let contender = dir.path().join("contender");
	fs::write(&contender, GOOD).unwrap();
	let mut second = start(&target, &contender);
	finish(&mut second, "cannot acquire update lock");
	assert_eq!(fs::read(target.with_extension("old")).unwrap(), OLD);
	assert_eq!(fs::read(&target).unwrap(), fs::read(&payload).unwrap());
	fs::write(target.with_extension("release"), []).unwrap();
	finish(&mut first, "INSTALL_OK");
	no_staging_files(dir.path());
	// A later process must acquire the released lock and complete normally.
	let mut retry = start(&target, &contender);
	finish(&mut retry, "INSTALL_OK");
	assert_eq!(fs::read(&target).unwrap(), GOOD);
	no_staging_files(dir.path());
}
