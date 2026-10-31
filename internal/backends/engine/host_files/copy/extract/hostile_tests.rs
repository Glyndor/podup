//! Archives a compromised container could hand back to `cp`, built to make
//! the extractor write, re-mode or link outside the destination.
//!
//! Every case builds the tar by hand and checks one invariant: the tree next
//! to the destination is byte-for-byte, mode-for-mode and link-for-link what
//! it was. Whether the extraction itself ends in an error is not the point,
//! so most cases ignore the result and only a panic or a change outside fails.

use super::archive::{extract_archive, extract_tar_guarded};
use super::hostile_support::{archive, dir, file, hardlink, symlink, Arena};

fn run(arena: &Arena, bytes: &[u8]) {
	let _ = extract_tar_guarded(bytes, &arena.dest);
}

#[test]
fn parent_traversal_in_every_spelling_writes_nothing_outside() {
	for name in [
		"../outside/pwn",
		"a/../../outside/pwn",
		"a/b/../../../outside/pwn",
		"./../outside/pwn",
		"a/./../../outside/pwn",
	] {
		let arena = Arena::new();
		run(&arena, &archive(&[file(name, b"pwn", 0o644)]));
		arena.assert_outside_untouched(name);
		assert!(!arena.outside.join("pwn").exists(), "{name} escaped");
	}
}

#[test]
fn an_absolute_entry_name_cannot_overwrite_a_host_file() {
	let arena = Arena::new();
	let abs = format!("{}/secret", arena.outside_str());
	run(&arena, &archive(&[file(&abs, b"pwn", 0o666)]));
	arena.assert_outside_untouched("absolute name");
}

#[test]
fn a_file_cannot_be_written_through_an_absolute_symlink() {
	let arena = Arena::new();
	let out = arena.outside_str();
	let tar = archive(&[symlink("link", &out), file("link/pwn", b"pwn", 0o644)]);
	run(&arena, &tar);
	arena.assert_outside_untouched("write through absolute symlink");
}

#[test]
fn a_file_cannot_be_written_through_a_relative_symlink() {
	let arena = Arena::new();
	let tar = archive(&[
		symlink("link", "../outside"),
		file("link/pwn", b"pwn", 0o644),
		file("link/dir/inner", b"pwn", 0o644),
	]);
	run(&arena, &tar);
	arena.assert_outside_untouched("write through relative symlink");
}

#[test]
fn a_directory_replaced_by_a_symlink_does_not_redirect_later_entries() {
	let arena = Arena::new();
	let out = arena.outside_str();
	let tar = archive(&[
		dir("d/", 0o755),
		symlink("d", &out),
		file("d/pwn", b"pwn", 0o644),
	]);
	run(&arena, &tar);
	arena.assert_outside_untouched("dir swapped for symlink");
}

#[test]
fn a_directory_entry_named_like_a_symlink_cannot_re_mode_its_target() {
	// `create_dir` fails with EEXIST on the link, the extractor accepts it
	// because the target is a directory, and the mode that follows is applied
	// through the link.
	let arena = Arena::new();
	let out = arena.outside_str();
	for mode in [0o777, 0o000, 0o4777] {
		let tar = archive(&[symlink("d", &format!("{out}/dir")), dir("d/", mode)]);
		run(&arena, &tar);
		arena.assert_outside_untouched(&format!("dir entry over symlink, mode {mode:o}"));
		let _ = std::fs::remove_file(arena.dest.join("d"));
	}
}

#[test]
fn a_file_replaced_by_a_symlink_then_rewritten_leaves_the_target_alone() {
	let arena = Arena::new();
	let out = arena.outside_str();
	let tar = archive(&[
		file("f", b"first", 0o644),
		symlink("f", &format!("{out}/secret")),
		file("f", b"pwn", 0o666),
	]);
	run(&arena, &tar);
	arena.assert_outside_untouched("file, symlink, file under one name");
}

#[test]
fn a_hardlink_to_an_absolute_host_file_cannot_be_written_through() {
	let arena = Arena::new();
	let target = format!("{}/secret", arena.outside_str());
	let tar = archive(&[hardlink("hl", &target), file("hl", b"pwn", 0o666)]);
	run(&arena, &tar);
	arena.assert_outside_untouched("hardlink to absolute path");
}

#[test]
fn a_hardlink_to_a_parent_relative_host_file_cannot_be_written_through() {
	let arena = Arena::new();
	let tar = archive(&[
		hardlink("hl", "../outside/secret"),
		file("hl", b"pwn", 0o666),
	]);
	run(&arena, &tar);
	arena.assert_outside_untouched("hardlink with ..");
}

#[test]
fn a_hardlink_reached_through_a_planted_symlink_directory_is_not_made() {
	let arena = Arena::new();
	let out = arena.outside_str();
	let tar = archive(&[
		symlink("sub", &out),
		hardlink("h", "sub/secret"),
		file("h", b"pwn", 0o666),
	]);
	run(&arena, &tar);
	arena.assert_outside_untouched("hardlink via symlinked directory");
}

#[test]
fn a_link_planted_by_one_copy_is_not_walked_by_the_next() {
	let arena = Arena::new();
	let out = arena.outside_str();
	run(&arena, &archive(&[symlink("keys", &out)]));
	assert!(arena.dest.join("keys").is_symlink(), "first copy plants it");
	run(
		&arena,
		&archive(&[file("keys/authorized_keys", b"pwn", 0o600)]),
	);
	run(&arena, &archive(&[dir("keys/", 0o777)]));
	arena.assert_outside_untouched("second copy through a planted link");
}

#[test]
fn a_wrapper_directory_holding_itself_does_not_panic_or_escape() {
	let arena = Arena::new();
	let dst = arena.dest.join("new");
	let tar = archive(&[
		dir("w/", 0o755),
		dir("w/w/", 0o755),
		file("w/w/x", b"x", 0o644),
	]);
	let _ = extract_archive(&tar, &dst);
	arena.assert_outside_untouched("wrapper inside wrapper");
}

#[test]
fn a_symlink_only_archive_into_a_missing_file_destination_creates_nothing() {
	let arena = Arena::new();
	let dst = arena.dest.join("out.txt");
	let tar = archive(&[symlink("x", "/etc/passwd")]);
	assert!(extract_archive(&tar, &dst).is_err());
	assert!(
		std::fs::symlink_metadata(&dst).is_err(),
		"something was created"
	);
}
