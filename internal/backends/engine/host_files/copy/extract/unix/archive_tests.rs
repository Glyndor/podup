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

#[cfg(unix)]
#[test]
fn hardlinks_to_preexisting_files_are_rejected_without_changing_permissions() {
	use super::super::hostile_support::raw;
	let tmp = tempfile::tempdir().unwrap();
	let dst = tmp.path().join("dest");
	std::fs::create_dir(&dst).unwrap();
	let credential = dst.join("credencial");
	let outside = tmp.path().join("another-name");
	std::fs::write(&credential, b"original").unwrap();
	std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
	std::fs::hard_link(&credential, &outside).unwrap();
	let root = Directory::open_host(&dst).unwrap();
	let bytes = archive(&[raw(
		b"copia",
		tar::EntryType::Link,
		b"credencial",
		0o644,
		b"",
	)]);
	let result = extract(&bytes[..], &root);
	for path in [&credential, &outside] {
		let metadata = std::fs::metadata(path).unwrap();
		assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
		assert_eq!(metadata.nlink(), 2);
		assert_eq!(std::fs::read(path).unwrap(), b"original");
	}
	assert!(std::fs::symlink_metadata(dst.join("copia")).is_err());
	let error = result.expect_err("a hard link requires a file from this extraction");
	assert!(error.to_string().contains("credencial"), "{error}");
}

#[cfg(unix)]
#[test]
fn hardlinks_to_files_created_by_the_current_extraction_share_the_inode() {
	use super::super::hostile_support::raw;
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let bytes = archive(&[
		file("a", b"content", 0o600),
		raw(b"b", tar::EntryType::Link, b"a", 0o644, b""),
	]);
	extract(&bytes[..], &root).unwrap();
	let original = std::fs::metadata(tmp.path().join("a")).unwrap();
	let linked = std::fs::metadata(tmp.path().join("b")).unwrap();
	assert_eq!(original.dev(), linked.dev());
	assert_eq!(original.ino(), linked.ino());
	assert_eq!(original.permissions().mode() & 0o7777, 0o644);
	assert_eq!(linked.permissions().mode() & 0o7777, 0o644);
	assert_eq!(std::fs::read(tmp.path().join("b")).unwrap(), b"content");
}

#[cfg(unix)]
#[test]
fn readonly_directories_receive_their_final_mode_after_their_contents() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let bytes = archive(&[
		dir("datos/", 0o555),
		file("datos/contenido", b"content", 0o644),
	]);
	let result = extract(&bytes[..], &root);
	let metadata = std::fs::metadata(tmp.path().join("datos")).unwrap();
	let content = std::fs::read(tmp.path().join("datos/contenido"));
	std::fs::set_permissions(
		tmp.path().join("datos"),
		std::fs::Permissions::from_mode(0o755),
	)
	.unwrap();
	result.unwrap();
	assert_eq!(metadata.permissions().mode() & 0o7777, 0o555);
	assert_eq!(content.unwrap(), b"content");
}

#[cfg(unix)]
#[test]
fn repeated_directory_headers_keep_the_last_mode_with_nested_contents() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let bytes = archive(&[
		dir("datos/sub/", 0o500),
		dir("datos/", 0o555),
		file("datos/sub/first", b"first", 0o600),
		dir("datos/sub/", 0o555),
		dir("datos/", 0o500),
		file("datos/sub/second", b"second", 0o644),
	]);
	let result = extract(&bytes[..], &root);
	let outer = std::fs::metadata(tmp.path().join("datos")).unwrap();
	let inner = std::fs::metadata(tmp.path().join("datos/sub")).unwrap();
	let content = std::fs::read(tmp.path().join("datos/sub/second"));
	for path in ["datos", "datos/sub"] {
		std::fs::set_permissions(
			tmp.path().join(path),
			std::fs::Permissions::from_mode(0o755),
		)
		.unwrap();
	}
	result.unwrap();
	assert_eq!(outer.permissions().mode() & 0o7777, 0o500);
	assert_eq!(inner.permissions().mode() & 0o7777, 0o555);
	assert_eq!(content.unwrap(), b"second");
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

/// The name of a hard link is opened again to give it its mode. If another
/// regular file has been put under that name by then, the mode must not be
/// applied to it: the file may have other names outside the destination.
#[cfg(unix)]
#[test]
fn a_hard_link_replaced_before_its_mode_is_applied_is_not_changed() {
	use super::super::hostile_support::raw;
	let tmp = tempfile::tempdir().unwrap();
	let dst = tmp.path().join("dest");
	std::fs::create_dir(&dst).unwrap();
	let outside = tmp.path().join("outside");
	std::fs::write(&outside, b"private").unwrap();
	std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o600)).unwrap();
	let root = Directory::open_host(&dst).unwrap();
	let linked = dst.join("b");
	let (swap_from, swap_to) = (outside.clone(), linked.clone());
	super::directory::before_reopen::set(move || {
		std::fs::remove_file(&swap_to).unwrap();
		std::fs::hard_link(&swap_from, &swap_to).unwrap();
	});
	let bytes = archive(&[
		file("a", b"content", 0o600),
		raw(b"b", tar::EntryType::Link, b"a", 0o644, b""),
	]);
	let result = extract(&bytes[..], &root);
	assert_eq!(
		std::fs::metadata(&outside).unwrap().permissions().mode() & 0o7777,
		0o600,
		"the mode of a file the extraction did not create was changed"
	);
	let error = result.expect_err("a replaced link must stop the extraction");
	assert!(error.to_string().contains("changed"), "{error}");
}
