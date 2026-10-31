//! Ordinary archive semantics and the buffered file destination.

use std::os::unix::fs::{symlink, PermissionsExt};

use super::super::hostile_support::{archive, dir, file};
use super::{extract, single_file, Directory};

#[path = "archive_tests/links.rs"]
mod links;
#[path = "archive_tests/modes.rs"]
mod modes;
#[path = "archive_tests/sparse.rs"]
mod sparse;

#[cfg(unix)]
#[test]
fn buffered_file_uses_its_pinned_parent_after_a_destination_swap() {
	let tmp = tempfile::tempdir().unwrap();
	let dst = tmp.path().join("dest");
	let original = tmp.path().join("original");
	let outside = tmp.path().join("outside");
	std::fs::create_dir(&dst).unwrap();
	std::fs::create_dir(&outside).unwrap();
	let parent = Directory::open_host(&dst).unwrap();
	std::fs::rename(&dst, &original).unwrap();
	symlink(&outside, &dst).unwrap();
	let bytes = archive(&[file("ignored-name", b"copied", 0o644)]);
	single_file(
		&bytes[..],
		&parent,
		std::ffi::OsStr::new("result"),
		&dst.join("result"),
	)
	.unwrap();
	assert_eq!(std::fs::read(original.join("result")).unwrap(), b"copied");
	assert!(!outside.join("result").exists());
}

#[cfg(unix)]
#[cfg(any(
	target_os = "linux",
	target_os = "android",
	target_os = "freebsd",
	target_vendor = "apple",
	target_os = "netbsd",
	target_os = "solaris",
	target_os = "illumos"
))]
#[test]
fn host_resolution_preserves_execute_only_ancestor_access() {
	let tmp = tempfile::tempdir().unwrap();
	let ancestor = tmp.path().join("search-only");
	let dst = ancestor.join("dest");
	std::fs::create_dir_all(&dst).unwrap();
	std::fs::set_permissions(&ancestor, std::fs::Permissions::from_mode(0o111)).unwrap();
	let root = Directory::open_host(&dst);
	std::fs::set_permissions(&ancestor, std::fs::Permissions::from_mode(0o700)).unwrap();
	let root = root.unwrap();
	extract(&archive(&[file("copied", b"content", 0o644)])[..], &root).unwrap();
	assert_eq!(std::fs::read(dst.join("copied")).unwrap(), b"content");
}

/// Open descriptors of this process, counted from /proc.
fn open_descriptors() -> usize {
	std::fs::read_dir("/proc/self/fd")
		.map(|d| d.count())
		.unwrap_or(0)
}

/// A reader that notes how many descriptors were open at its first and its last
/// read, which bracket the extraction.
struct WatchDescriptors<R> {
	inner: R,
	first: Option<usize>,
	last: usize,
}

impl<R: std::io::Read> std::io::Read for WatchDescriptors<R> {
	fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
		let open = open_descriptors();
		self.first.get_or_insert(open);
		self.last = open;
		self.inner.read(buf)
	}
}

/// Extraction keeps what it has to know about the files and directories it
/// created as identities, not as open descriptors: with one descriptor kept per
/// file, a tree of a few thousand files runs into the limit of a process.
#[cfg(target_os = "linux")]
#[test]
fn extracting_many_files_does_not_keep_a_descriptor_for_each() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let mut parts = vec![dir("tree/", 0o755)];
	for i in 0..1500 {
		parts.push(file(&format!("tree/f{i}"), b"x", 0o644));
	}
	let bytes = archive(&parts);
	let mut watched = WatchDescriptors {
		inner: &bytes[..],
		first: None,
		last: 0,
	};
	extract(&mut watched, &root).unwrap();
	let first = watched.first.expect("the archive was read");
	// Other tests open and close descriptors while this one runs, so the bound
	// is far below the 1500 a descriptor per file would add.
	assert!(
		watched.last < first + 300,
		"descriptors grew from {first} to {} while extracting",
		watched.last
	);
}
