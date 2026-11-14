//! The archive `cp` reads out of a container, extracted on Windows.
//!
//! The Unix extractor has its own hostile battery (`hostile_tests.rs`, Unix
//! only); on Windows `extract_tar_guarded` relies on the tar crate's
//! `unpack_in`, which nothing exercised with a hostile archive. These cases
//! use Windows spellings (`..\`, drive letters, `\\?\`) and run on every
//! platform, so the Windows extractor is covered on the Windows runner and the
//! Unix one meets the same entries here. Whatever the extractor answers (an
//! error, or skipping the entry), nothing may be written outside the
//! destination.

use std::path::Path;

use tar::{EntryType, Header};

use super::archive::extract_tar_guarded;

/// One 512-byte header written field by field, so `..`, absolute names and
/// drive letters survive (the safe setters refuse them), then the body padded
/// to a block.
fn raw(name: &[u8], kind: EntryType, link: &[u8], data: &[u8]) -> Vec<u8> {
	let mut header = Header::new_gnu();
	header.set_entry_type(kind);
	header.set_mode(0o644);
	header.set_size(data.len() as u64);
	{
		let gnu = header.as_gnu_mut().expect("gnu header");
		gnu.name[..name.len()].copy_from_slice(name);
		gnu.linkname[..link.len()].copy_from_slice(link);
	}
	header.set_cksum();
	let mut out = header.as_bytes().to_vec();
	out.extend_from_slice(data);
	out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
	out
}

/// The entries, closed with the two zero blocks.
fn archive(parts: &[Vec<u8>]) -> Vec<u8> {
	let mut out = parts.concat();
	out.resize(out.len() + 1024, 0);
	out
}

/// Every file below `root`, relative, sorted.
fn listing(root: &Path) -> Vec<String> {
	let mut out = Vec::new();
	let mut stack = vec![root.to_path_buf()];
	while let Some(dir) = stack.pop() {
		for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
			let path = entry.path();
			if path.is_dir() {
				stack.push(path.clone());
			}
			out.push(path.strip_prefix(root).unwrap().display().to_string());
		}
	}
	out.sort();
	out
}

/// Extract `bytes` into a fresh destination next to an `outside` directory
/// holding one marker file, and assert `outside` is exactly as it was.
fn assert_contained(what: &str, bytes: &[u8]) {
	let root = tempfile::tempdir().unwrap();
	let dest = root.path().join("dest");
	let outside = root.path().join("outside");
	std::fs::create_dir(&dest).unwrap();
	std::fs::create_dir(&outside).unwrap();
	std::fs::write(outside.join("marker"), b"keep").unwrap();
	let before = listing(&outside);

	// An error is a fine answer; what matters is where the bytes went.
	let _ = extract_tar_guarded(std::io::Cursor::new(bytes), &dest);

	assert_eq!(
		listing(&outside),
		before,
		"{what}: wrote outside the destination"
	);
	assert_eq!(
		std::fs::read(outside.join("marker")).unwrap(),
		b"keep",
		"{what}"
	);
	assert!(
		!root.path().join("pwned").exists(),
		"{what}: wrote next to the destination"
	);
}

#[test]
fn a_parent_component_does_not_leave_the_destination() {
	for name in [
		"..\\pwned",
		"../pwned",
		"a\\..\\..\\pwned",
		"..\\outside\\pwned",
	] {
		let bytes = archive(&[raw(name.as_bytes(), EntryType::Regular, b"", b"x")]);
		assert_contained(name, &bytes);
	}
}

#[test]
fn an_absolute_or_drive_path_does_not_leave_the_destination() {
	let root = tempfile::tempdir().unwrap();
	let absolute = root.path().join("pwned").display().to_string();
	for name in [
		absolute.as_str(),
		"C:\\pwned",
		"\\pwned",
		"/pwned",
		"\\\\?\\C:\\pwned",
	] {
		let bytes = archive(&[raw(name.as_bytes(), EntryType::Regular, b"", b"x")]);
		assert_contained(name, &bytes);
	}
	assert!(!Path::new(&absolute).exists(), "wrote to {absolute}");
}

/// A link to a directory outside, then a file through it: the classic
/// two-step escape. A link the account may not create (no symlink privilege)
/// is an error, which is also contained.
#[test]
fn a_file_written_through_a_link_does_not_leave_the_destination() {
	for target in ["..\\outside", "../outside", "C:\\"] {
		let bytes = archive(&[
			raw(b"d", EntryType::Symlink, target.as_bytes(), b""),
			raw(b"d\\pwned", EntryType::Regular, b"", b"x"),
			raw(b"d/pwned", EntryType::Regular, b"", b"x"),
		]);
		assert_contained(&format!("symlink to {target}"), &bytes);
	}
}

#[test]
fn a_hard_link_to_a_file_outside_is_not_followed() {
	for target in ["..\\outside\\marker", "../outside/marker"] {
		let bytes = archive(&[raw(b"h", EntryType::Link, target.as_bytes(), b"")]);
		assert_contained(&format!("hard link to {target}"), &bytes);
	}
}
