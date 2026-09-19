//! Tests for #1764: a symlink at ANY component of a container→host `cp`
//! destination must be refused, not only one at the last component.
//!
//! `lstat` declines to follow the last component of a path and follows every
//! earlier one, so `real/mid/inner` with `mid -> victim` classified as the
//! directory `victim/inner` and the bytes landed there. A trailing separator
//! did the same to the last component: `lstat("last/")` follows `last`.
//!
//! Both gates are exercised through the entry points production uses:
//! `cp_destination_kind` (the routing in `cp_from_container`) and
//! `extract_archive` (the buffered branch). The `extract_archive` cases
//! assert the harm rather than the verdict: nothing may appear under the
//! victim directory.

#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

#[cfg(unix)]
use super::archive::extract_archive;
use super::{cp_destination_kind, CpDestinationKind};

/// A tree with no symlink in it, under a base that has none either. The base
/// is canonicalised because the platform's temp directory may itself sit
/// behind a link (`/var` on macOS), and that link is not what is under test.
///
/// ```text
/// base/real/sub/leaf/      directories
/// base/real/file           regular file
/// base/victim/inner/       where a followed link would land the bytes
/// ```
fn tree() -> (tempfile::TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("tempdir");
	let base = dir.path().canonicalize().expect("canonicalize");
	std::fs::create_dir_all(base.join("real/sub/leaf")).expect("mkdir");
	std::fs::write(base.join("real/file"), b"x").expect("write");
	std::fs::create_dir_all(base.join("victim/inner")).expect("mkdir");
	(dir, base)
}

/// `tree()` plus the three links under test, each pointing at `victim`:
/// `first` directly under the base, `real/mid` in the middle of a path and
/// `real/last` to be named as the destination itself.
#[cfg(unix)]
fn tree_with_links() -> (tempfile::TempDir, PathBuf) {
	let (dir, base) = tree();
	for link in ["first", "real/mid", "real/last"] {
		std::os::unix::fs::symlink(base.join("victim"), base.join(link)).expect("symlink");
	}
	(dir, base)
}

/// The same location spelled relative to the working directory. `..` is
/// resolved physically, so more of them than the working directory is deep
/// always reach the root, and the test never has to change directory.
/// `pub(super)` so the trusted-root tests in the sibling module can reuse it
/// instead of re-implementing the same walk.
#[cfg(unix)]
pub(super) fn relative_to_cwd(absolute: &Path) -> PathBuf {
	let mut out = PathBuf::new();
	for _ in 0..64 {
		out.push("..");
	}
	out.join(absolute.strip_prefix("/").expect("absolute"))
}

/// An uncompressed tar holding one regular file, the shape libpod returns
/// for a single-file source.
#[cfg(unix)]
fn single_file_tar(name: &str, body: &[u8]) -> Vec<u8> {
	let mut builder = tar::Builder::new(Vec::new());
	let mut header = tar::Header::new_gnu();
	header.set_size(body.len() as u64);
	header.set_mode(0o644);
	header.set_entry_type(tar::EntryType::Regular);
	header.set_cksum();
	builder
		.append_data(&mut header, name, body)
		.expect("append");
	builder.into_inner().expect("finish")
}

#[cfg(unix)]
fn is_refused(dst: &Path) -> bool {
	matches!(cp_destination_kind(dst), CpDestinationKind::Symlink)
}

/// Everything under `dir`, so a test can assert that nothing landed there.
#[cfg(unix)]
fn entries(dir: &Path) -> usize {
	std::fs::read_dir(dir).expect("read_dir").count()
}

#[cfg(unix)]
#[test]
fn routing_refuses_a_symlink_at_an_intermediate_component() {
	let (_dir, base) = tree_with_links();
	assert!(
		is_refused(&base.join("real/mid/inner")),
		"intermediate component: real/mid is a symlink and real/mid/inner was routed as a destination",
	);
}

#[cfg(unix)]
#[test]
fn routing_refuses_a_symlink_at_the_first_component() {
	let (_dir, base) = tree_with_links();
	assert!(
		is_refused(&base.join("first/inner")),
		"first component: first is a symlink and first/inner was routed as a destination",
	);
}

#[cfg(unix)]
#[test]
fn routing_refuses_a_symlink_at_the_last_component() {
	let (_dir, base) = tree_with_links();
	assert!(
		is_refused(&base.join("real/last")),
		"last component: real/last is a symlink and was routed as a destination",
	);
}

#[cfg(unix)]
#[test]
fn routing_refuses_a_last_component_symlink_named_with_a_trailing_separator() {
	let (_dir, base) = tree_with_links();
	let mut dst = base.join("real/last").into_os_string();
	dst.push("/");
	assert!(
		is_refused(Path::new(&dst)),
		"trailing separator: real/last/ names a symlink and was routed as a destination",
	);
}

/// The whole table at once, asserting how many destinations were refused and
/// how many were accepted. A guard that refused everything, or nothing, would
/// satisfy any single case above or below; it cannot satisfy both counts.
#[cfg(unix)]
#[test]
fn routing_refuses_exactly_the_destinations_that_cross_a_symlink() {
	let (_dir, base) = tree_with_links();
	let mut trailing = base.join("real/last").into_os_string();
	trailing.push("/");
	let crossing: Vec<(&str, PathBuf)> = vec![
		("first component", base.join("first/inner")),
		("intermediate component", base.join("real/mid/inner")),
		(
			"intermediate component, missing tail",
			base.join("real/mid/absent/file"),
		),
		(
			"intermediate component, then ..",
			base.join("real/mid/../victim/inner"),
		),
		(
			"intermediate component, relative",
			relative_to_cwd(&base.join("real/mid/inner")),
		),
		("last component", base.join("real/last")),
		(
			"last component, trailing separator",
			PathBuf::from(trailing),
		),
	];
	let clean: Vec<(&str, PathBuf)> = vec![
		("existing directory", base.join("real/sub/leaf")),
		("missing leaf", base.join("real/sub/absent")),
		("missing parent", base.join("real/absent/deeper/file")),
		("existing file", base.join("real/file")),
		("below a regular file", base.join("real/file/x")),
		("through ..", base.join("real/sub/../sub/leaf")),
		(
			"existing directory, relative",
			relative_to_cwd(&base.join("real/sub/leaf")),
		),
	];

	let accepted_crossing: Vec<&str> = crossing
		.iter()
		.filter(|(_, dst)| !is_refused(dst))
		.map(|(label, _)| *label)
		.collect();
	let refused_clean: Vec<&str> = clean
		.iter()
		.filter(|(_, dst)| is_refused(dst))
		.map(|(label, _)| *label)
		.collect();

	assert_eq!(
		crossing.len() - accepted_crossing.len(),
		7,
		"destinations crossing a symlink that were accepted: {accepted_crossing:?}",
	);
	assert_eq!(
		clean.len() - refused_clean.len(),
		7,
		"symlink-free destinations that were refused: {refused_clean:?}",
	);
}

/// The acceptance half, on every platform: a tree with no symlink keeps the
/// classification it had before the walk existed.
#[test]
fn routing_keeps_classifying_a_tree_without_symlinks() {
	let (_dir, base) = tree();
	assert!(matches!(
		cp_destination_kind(&base.join("real/sub/leaf")),
		CpDestinationKind::Directory
	));
	assert!(matches!(
		cp_destination_kind(&base.join("real/sub/../sub/leaf")),
		CpDestinationKind::Directory
	));
	for dst in ["real/sub/absent", "real/absent/deeper/file", "real/file"] {
		assert!(
			matches!(
				cp_destination_kind(&base.join(dst)),
				CpDestinationKind::NotADirectory
			),
			"{dst} must stay a buffered destination",
		);
	}
}

#[cfg(unix)]
#[test]
fn extract_refuses_a_file_destination_behind_an_intermediate_symlink() {
	let (_dir, base) = tree_with_links();
	let tar = single_file_tar("payload", b"bytes");

	let err = extract_archive(&tar, &base.join("real/mid/stolen"))
		.expect_err("intermediate component: real/mid is a symlink and the copy went through");
	assert!(
		err.to_string().contains("refusing symlink destination"),
		"intermediate component: refused for another reason: {err}",
	);
	assert_eq!(
		entries(&base.join("victim")),
		1,
		"intermediate component: the payload landed in the link target",
	);

	// Same shape, one directory over, no link: must land.
	extract_archive(&tar, &base.join("real/sub/kept")).expect("symlink-free destination");
	assert_eq!(
		std::fs::read(base.join("real/sub/kept")).expect("read"),
		b"bytes"
	);
}

#[cfg(unix)]
#[test]
fn extract_refuses_a_directory_destination_behind_an_intermediate_symlink() {
	let (_dir, base) = tree_with_links();
	let tar = single_file_tar("payload", b"bytes");

	let err = extract_archive(&tar, &base.join("real/mid/inner"))
		.expect_err("intermediate component: real/mid is a symlink and the copy went through");
	assert!(
		err.to_string().contains("refusing symlink destination"),
		"intermediate component: refused for another reason: {err}",
	);
	assert_eq!(
		entries(&base.join("victim/inner")),
		0,
		"intermediate component: the payload landed in the link target",
	);

	extract_archive(&tar, &base.join("real/sub/leaf")).expect("symlink-free destination");
	assert_eq!(
		std::fs::read(base.join("real/sub/leaf/payload")).expect("read"),
		b"bytes"
	);
}

#[cfg(unix)]
#[test]
fn extract_refuses_a_symlink_at_the_last_component() {
	let (_dir, base) = tree_with_links();
	let tar = single_file_tar("payload", b"bytes");

	let err = extract_archive(&tar, &base.join("real/last"))
		.expect_err("last component: real/last is a symlink and the copy went through");
	assert!(
		err.to_string().contains("refusing symlink destination"),
		"last component: refused for another reason: {err}",
	);
	assert_eq!(
		entries(&base.join("victim")),
		1,
		"last component: the payload landed in the link target",
	);
}

#[cfg(unix)]
#[test]
fn extract_refuses_a_last_component_symlink_named_with_a_trailing_separator() {
	let (_dir, base) = tree_with_links();
	let tar = single_file_tar("payload", b"bytes");
	let mut dst = base.join("real/last").into_os_string();
	dst.push("/");

	let err = extract_archive(&tar, Path::new(&dst))
		.expect_err("trailing separator: real/last/ names a symlink and the copy went through");
	assert!(
		err.to_string().contains("refusing symlink destination"),
		"trailing separator: refused for another reason: {err}",
	);
	assert_eq!(
		entries(&base.join("victim")),
		1,
		"trailing separator: the payload landed in the link target",
	);
}

/// The operator has to be told which part of the path to spell differently:
/// on a system where `/tmp` or `/home` is a link, the destination they typed
/// does not look like a symlink to them.
#[cfg(unix)]
#[test]
fn the_refusal_names_the_component_that_is_a_symlink() {
	let (_dir, base) = tree_with_links();
	let tar = single_file_tar("payload", b"bytes");

	let err = extract_archive(&tar, &base.join("real/mid/inner")).expect_err("refused");
	let named = format!("{} is a symlink", base.join("real/mid").display());
	assert!(
		err.to_string().contains(&named),
		"the refusal does not name real/mid: {err}",
	);
}

/// A component that can be neither confirmed nor ruled out is refused. A name
/// past `NAME_MAX` fails `lstat` with something other than "not found"
/// whoever runs the suite, which a permission bit would not: root reads
/// through `chmod 000`.
#[cfg(unix)]
#[test]
fn a_component_that_cannot_be_inspected_is_refused() {
	let (_dir, base) = tree();
	let tar = single_file_tar("payload", b"bytes");

	let unreadable = base.join("real").join("x".repeat(300)).join("file");
	assert!(is_refused(&unreadable));
	let err = extract_archive(&tar, &unreadable).expect_err("refused");
	assert!(
		err.to_string().contains("cannot inspect"),
		"refused for another reason: {err}",
	);

	// Same shape just inside the limit: merely missing, so not refused here.
	let missing = base.join("real").join("x".repeat(200)).join("file");
	assert!(!is_refused(&missing));
}
