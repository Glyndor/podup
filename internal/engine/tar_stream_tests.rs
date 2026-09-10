//! The archive dialect Podman accepts, measured on a real sparse file.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use super::builder;

/// Write a file that is `hole` bytes of nothing followed by `tail`.
///
/// Returns whether a sparse entry can be expected of the crate for this file:
/// the hole has to be real, and the target has to be one the crate looks on.
/// A test that packs a file the crate will never call sparse and then finds no
/// sparse entry has measured nothing, so the positive control is conditioned
/// on this answer rather than assuming it.
fn write_sparse(path: &Path, hole: u64, tail: &[u8]) -> bool {
	let mut f = File::create(path).expect("create");
	f.set_len(hole).expect("set_len");
	// `set_len` does not move the cursor. Without the seek the tail lands at
	// offset 0, the file keeps its `hole` length and the assertions below read
	// a file that is not the one they describe.
	f.seek(SeekFrom::Start(hole))
		.expect("seek to the end of the hole");
	f.write_all(tail).expect("write tail");
	f.sync_all().expect("sync");
	drop(f);
	on_disk_is_smaller_than_apparent(path)
}

/// The three targets where the crate looks for holes at all.
///
/// `find_sparse_entries` in the `tar` crate is `cfg`'d to return "not sparse"
/// on every other target, so this list is not about which filesystem can store
/// a hole. macOS is the one that makes the difference visible: it is Unix, APFS
/// really does leave the range unallocated, so a block count would report a
/// sparse file, and the crate would still never write a sparse entry for it.
/// Keying the answer on `cfg(unix)` would therefore have failed the positive
/// control on the macOS runner for a reason that has nothing to do with this
/// fix. Mirror the crate's own `cfg` instead.
#[cfg(any(target_os = "android", target_os = "freebsd", target_os = "linux"))]
fn on_disk_is_smaller_than_apparent(path: &Path) -> bool {
	use std::os::unix::fs::MetadataExt;
	let meta = std::fs::metadata(path).expect("metadata");
	meta.blocks() * 512 < meta.size()
}

#[cfg(not(any(target_os = "android", target_os = "freebsd", target_os = "linux")))]
fn on_disk_is_smaller_than_apparent(_path: &Path) -> bool {
	false
}

/// Pack one file and report the entry types the archive carries.
fn entry_types<F>(make: F, path: &Path, name: &str) -> Vec<tar::EntryType>
where
	F: FnOnce(Vec<u8>) -> tar::Builder<Vec<u8>>,
{
	let mut tar = make(Vec::new());
	tar.follow_symlinks(false);
	tar.append_path_with_name(path, name).expect("append");
	let bytes = tar.into_inner().expect("finish");
	tar::Archive::new(&bytes[..])
		.entries()
		.expect("entries")
		.map(|e| e.expect("entry").header().entry_type())
		.collect()
}

/// The defect: the crate's own default writes the type Podman refuses.
///
/// This is the positive control. Without it the test below is vacuous on any
/// platform where the crate never detects a hole, and it would stay green with
/// the fix reverted while reporting that it had checked something.
#[test]
fn the_crate_default_emits_the_type_podman_refuses_when_the_hole_is_real() {
	let dir = tempfile::tempdir().expect("tempdir");
	let path = dir.path().join("holey.bin");
	if !write_sparse(&path, 1 << 20, b"data") {
		// Either the filesystem stored the zeroes or this target is one the
		// crate never inspects, so the control cannot be established here. Say
		// so rather than pass quietly: the test below still checks the fix, it
		// just cannot show on this runner that the fix was needed.
		eprintln!("tar_stream: no sparse entry is possible here, positive control not established");
		return;
	}
	let types = entry_types(tar::Builder::new, &path, "holey.bin");
	assert!(
		types.contains(&tar::EntryType::GNUSparse),
		"the default builder was expected to write a GNU sparse entry for a file \
		 with a 1 MiB hole; it wrote {types:?}. If the crate stopped emitting \
		 sparse entries the fix in tar_stream.rs may be obsolete, but check that \
		 before deleting it."
	);
}

/// The fix: our constructor never writes it, hole or no hole.
#[test]
fn our_builder_never_writes_a_gnu_sparse_entry() {
	let dir = tempfile::tempdir().expect("tempdir");
	let path = dir.path().join("holey.bin");
	let sparse = write_sparse(&path, 1 << 20, b"data");
	let types = entry_types(builder, &path, "holey.bin");
	assert!(
		!types.contains(&tar::EntryType::GNUSparse),
		"a sparse file ({sparse}) packed through engine::tar_stream::builder \
		 produced {types:?}; Podman answers a GNU sparse entry with \
		 `unhandled tar header type 83` on the build endpoint and \
		 `unrecognized Typeflag S` on the archive endpoint, and fails the whole \
		 transfer either way"
	);
}

/// Expanding the holes has to preserve the bytes, not only the entry type.
#[test]
fn the_expanded_entry_carries_every_byte_the_file_had() {
	let dir = tempfile::tempdir().expect("tempdir");
	let path = dir.path().join("holey.bin");
	write_sparse(&path, 1 << 16, b"tail");
	let mut tar = builder(Vec::new());
	tar.append_path_with_name(&path, "holey.bin")
		.expect("append");
	let bytes = tar.into_inner().expect("finish");

	let mut archive = tar::Archive::new(&bytes[..]);
	let mut entries = archive.entries().expect("entries");
	let mut entry = entries.next().expect("one entry").expect("entry");
	assert_eq!(entry.header().size().expect("size"), (1 << 16) + 4);
	let mut got = Vec::new();
	std::io::Read::read_to_end(&mut entry, &mut got).expect("read");
	assert_eq!(got.len(), (1 << 16) + 4, "the hole is stored as zeroes");
	assert_eq!(
		&got[(1 << 16)..],
		b"tail",
		"the tail survives the expansion"
	);
	assert!(
		got[..(1 << 16)].iter().all(|b| *b == 0),
		"the hole reads back as zeroes"
	);
}
