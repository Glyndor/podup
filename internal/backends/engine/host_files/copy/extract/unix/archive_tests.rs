//! Ordinary archive semantics and the buffered file destination.

use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

use super::super::hostile_support::{archive, dir, file, hardlink, symlink as tar_symlink};
use super::{extract, single_file, Directory};

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
#[test]
fn ordinary_entries_keep_contents_links_and_sanitized_modes() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let bytes = archive(&[
		dir("sub", 0o777),
		file("sub/file", b"copied", 0o6777),
		hardlink("hard", "sub/file"),
		tar_symlink("symbolic", "sub/file"),
		file("sub/file", b"replacement", 0o644),
	]);
	extract(&bytes[..], &root).unwrap();
	assert_eq!(
		std::fs::read(tmp.path().join("sub/file")).unwrap(),
		b"replacement"
	);
	assert_eq!(std::fs::read(tmp.path().join("hard")).unwrap(), b"copied");
	assert_eq!(
		std::fs::read_link(tmp.path().join("symbolic")).unwrap(),
		std::path::Path::new("sub/file")
	);
	let original = std::fs::metadata(tmp.path().join("hard")).unwrap();
	let replacement = std::fs::metadata(tmp.path().join("sub/file")).unwrap();
	assert_ne!(original.ino(), replacement.ino());
	assert_eq!(original.permissions().mode() & 0o7777, 0o644);
	assert_eq!(
		std::fs::metadata(tmp.path().join("sub"))
			.unwrap()
			.permissions()
			.mode() & 0o7777,
		0o755
	);
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

#[cfg(unix)]
#[test]
fn sparse_entries_preserve_logical_size_contents_and_holes() {
	use std::io::{Read, Seek, SeekFrom};
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let size = 4 * 1024 * 1024;
	let mut header = tar::Header::new_gnu();
	header.set_path("sparse").unwrap();
	header.set_entry_type(tar::EntryType::GNUSparse);
	header.set_size(4);
	header.set_mode(0o644);
	let gnu = header.as_gnu_mut().unwrap();
	gnu.set_real_size(size);
	gnu.sparse[0].set_offset(size - 4);
	gnu.sparse[0].set_length(4);
	header.set_cksum();
	let mut bytes = header.as_bytes().to_vec();
	bytes.extend_from_slice(b"data");
	bytes.resize(512 + 512 + 1024, 0);
	extract(&bytes[..], &root).unwrap();
	let mut output = std::fs::File::open(tmp.path().join("sparse")).unwrap();
	let metadata = output.metadata().unwrap();
	assert_eq!(metadata.len(), size);
	// Holes survive only on file systems that keep them; the macOS default
	// (APFS) allocates the whole file, so the size and content checks are the
	// part that holds everywhere.
	#[cfg(target_os = "linux")]
	assert!(
		metadata.blocks() * 512 < size / 2,
		"sparse data was materialized"
	);
	output.seek(SeekFrom::Start(size - 4)).unwrap();
	let mut tail = Vec::new();
	output.read_to_end(&mut tail).unwrap();
	assert_eq!(tail, b"data");
}

/// An entry may declare any logical size in a few bytes of header, so the size
/// is bounded before anything is read.
#[cfg(unix)]
#[test]
fn a_sparse_entry_declaring_an_absurd_size_is_refused_before_it_is_read() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let size: u64 = 1 << 50;
	let mut header = tar::Header::new_gnu();
	header.set_path("huge").unwrap();
	header.set_entry_type(tar::EntryType::GNUSparse);
	header.set_size(4);
	header.set_mode(0o644);
	let gnu = header.as_gnu_mut().unwrap();
	gnu.set_real_size(size);
	gnu.sparse[0].set_offset(size - 4);
	gnu.sparse[0].set_length(4);
	header.set_cksum();
	let mut bytes = header.as_bytes().to_vec();
	bytes.extend_from_slice(b"data");
	bytes.resize(512 + 512 + 1024, 0);
	let started = std::time::Instant::now();
	let err = extract(&bytes[..], &root).expect_err("an absurd size must be refused");
	assert!(
		started.elapsed().as_secs() < 5,
		"it must not read the zeros"
	);
	assert!(err.to_string().contains("limit"), "{err}");
}

#[cfg(unix)]
#[test]
fn symlinks_keep_the_archive_timestamp_without_remoding_the_target() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	extract(
		&archive(&[
			file("target", b"data", 0o600),
			tar_symlink("link", "target"),
		])[..],
		&root,
	)
	.unwrap();
	let target = std::fs::metadata(tmp.path().join("target")).unwrap();
	assert_eq!(target.permissions().mode() & 0o777, 0o600);
	let link = std::fs::symlink_metadata(tmp.path().join("link")).unwrap();
	assert_eq!(link.mtime(), 1);
}

#[cfg(unix)]
#[test]
fn hardlinks_preserve_files_without_read_or_write_permission() {
	use super::super::hostile_support::raw;
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let bytes = archive(&[
		file("original", b"data", 0),
		raw(b"hard", tar::EntryType::Link, b"original", 0, b""),
	]);
	extract(&bytes[..], &root).unwrap();
	let original = std::fs::metadata(tmp.path().join("original")).unwrap();
	let linked = std::fs::metadata(tmp.path().join("hard")).unwrap();
	assert_eq!(linked.ino(), original.ino());
	assert_eq!(linked.permissions().mode() & 0o777, 0);
	std::fs::set_permissions(
		tmp.path().join("original"),
		std::fs::Permissions::from_mode(0o600),
	)
	.unwrap();
	assert_eq!(std::fs::read(tmp.path().join("hard")).unwrap(), b"data");
}
