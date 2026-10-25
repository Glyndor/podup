//! The self-update writer in a directory somebody else can write to.
//!
//! `install_at` writes the new binary next to the old one and renames it into
//! place. In a shared install directory the sibling names are guessable, so
//! the cases here plant a link, a directory and a file on them and check that
//! the victim behind the link is never written and the new bytes still land.

use std::os::unix::fs::{MetadataExt, PermissionsExt};

use super::{install_at, write_temp};

fn victim(dir: &std::path::Path) -> std::path::PathBuf {
	let victim = dir.join("victim");
	std::fs::write(&victim, b"precious").expect("write victim");
	std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).expect("chmod");
	victim
}

#[test]
fn a_link_planted_on_the_temp_name_does_not_redirect_the_new_binary() {
	let dir = tempfile::tempdir().expect("tempdir");
	let victim = victim(dir.path());
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").expect("write target");
	let tmp = dir.path().join("podup.tmp");
	std::os::unix::fs::symlink(&victim, &tmp).expect("symlink");

	write_temp(&tmp, b"new binary", &target).expect("the stale link is replaced, not followed");

	assert_eq!(std::fs::read(&victim).expect("read"), b"precious");
	assert!(!std::fs::symlink_metadata(&tmp)
		.expect("lstat")
		.file_type()
		.is_symlink());
	assert_eq!(std::fs::read(&tmp).expect("read"), b"new binary");
}

#[test]
fn a_dangling_link_on_the_temp_name_does_not_create_its_target() {
	let dir = tempfile::tempdir().expect("tempdir");
	let elsewhere = dir.path().join("created-by-the-link");
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").expect("write target");
	let tmp = dir.path().join("podup.tmp");
	std::os::unix::fs::symlink(&elsewhere, &tmp).expect("symlink");

	write_temp(&tmp, b"new binary", &target).expect("write");

	assert!(!elsewhere.exists(), "the dangling link was followed");
}

#[test]
fn installing_over_a_link_target_replaces_the_link_and_spares_what_it_pointed_at() {
	let dir = tempfile::tempdir().expect("tempdir");
	let victim = victim(dir.path());
	let target = dir.path().join("podup");
	std::os::unix::fs::symlink(&victim, &target).expect("symlink");

	install_at(&target, b"new binary").expect("install");

	assert_eq!(std::fs::read(&victim).expect("read"), b"precious");
	assert_eq!(std::fs::read(&target).expect("read"), b"new binary");
}

#[test]
fn a_special_bit_on_a_linked_target_does_not_reach_the_new_binary() {
	let dir = tempfile::tempdir().expect("tempdir");
	let victim = victim(dir.path());
	std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o6777)).expect("chmod");
	if std::fs::metadata(&victim).expect("stat").mode() & 0o6000 == 0 {
		return;
	}
	let target = dir.path().join("podup");
	std::os::unix::fs::symlink(&victim, &target).expect("symlink");

	install_at(&target, b"new binary").expect("install");

	let mode = std::fs::metadata(&target).expect("stat").mode();
	assert_eq!(
		mode & 0o7000,
		0,
		"special bits rode in: {:o}",
		mode & 0o7777
	);
}

#[test]
fn a_directory_on_the_temp_name_is_an_error_and_not_a_write_inside_it() {
	let dir = tempfile::tempdir().expect("tempdir");
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").expect("write target");
	let tmp = dir.path().join("podup.tmp");
	std::fs::create_dir(&tmp).expect("mkdir");
	std::fs::write(tmp.join("keep"), b"keep").expect("write");

	let _ = write_temp(&tmp, b"new binary", &target);

	assert_eq!(std::fs::read(tmp.join("keep")).expect("read"), b"keep");
	assert_eq!(std::fs::read(&target).expect("read"), b"old");
}
