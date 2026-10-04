use super::{build_sync_tar, has_permitted_entry};
use crate::engine::watch::actions::entry_is_unsyncable;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs;
use std::io::Read;
use std::path::Path;
use tempfile::tempdir;

/// Drive the unified `build_sync_tar` with a filter closure, and return
/// the gzipped tar bytes together with the `SentEntry` list the packer
/// recorded. Used by tests that want to assert the recorded list
/// alongside the archive contents so a sabotage that drops an entry from
/// the tar but leaves it in the recorder (or vice versa) is caught.
fn sync_to_gz_with_sent(
	src: &Path,
	entry_name: &Path,
	skip: &dyn Fn(&Path) -> bool,
) -> std::io::Result<(Vec<u8>, Vec<crate::engine::copy::verify::SentEntry>)> {
	let buf = Vec::new();
	let encoder = GzEncoder::new(buf, Compression::default());
	let mut tar = tar::Builder::new(encoder);
	let mut sent = Vec::new();
	build_sync_tar(src, entry_name, &mut tar, &mut sent, skip)
		.map_err(|e| std::io::Error::other(e.to_string()))?;
	let encoder = tar.into_inner().map_err(std::io::Error::other)?;
	let bytes = encoder.finish().map_err(std::io::Error::other)?;
	Ok((bytes, sent))
}

/// Decode a gzipped tar and return `(path, kind)` for every entry that has
/// one (`Kind` is "file", "dir", or "link"). Ordering is the archive order,
/// which is the order the packer appended the entries.
fn tar_entries(gz: &[u8]) -> Vec<(String, &'static str)> {
	let mut decoder = GzDecoder::new(gz);
	let mut raw = Vec::new();
	decoder.read_to_end(&mut raw).unwrap();
	let mut archive = tar::Archive::new(&raw[..]);
	let mut out = Vec::new();
	for entry in archive.entries().unwrap() {
		let entry = entry.unwrap();
		let path = entry
			.path()
			.unwrap()
			.to_string_lossy()
			.replace('\\', "/")
			.to_string();
		let kind = if entry.header().entry_type().is_dir() {
			"dir"
		} else if entry.header().entry_type().is_symlink() {
			"link"
		} else if entry.header().entry_type().is_file() {
			"file"
		} else {
			continue;
		};
		out.push((path, kind));
	}
	out
}

// --- has_permitted_entry -----------------------------------------------

/// A single file whose entry name the filter rejects has nothing to
/// upload: no descendant walk, no root entry (a single file has no root
/// wrapper), just the filter's `true`. The caller can short-circuit
/// the upload and skip the `mkdir -p` and the PUT.
#[test]
fn has_permitted_entry_single_file_skip_returns_false() {
	let dir = tempdir().unwrap();
	let file = dir.path().join("solo.txt");
	fs::write(&file, b"x").unwrap();
	let skip = |name: &Path| name == Path::new("solo.txt");
	assert!(
		!has_permitted_entry(&file, Path::new("solo.txt"), &skip).unwrap(),
		"a single file the filter drops has no permitted entry"
	);
}

/// A directory whose root entry and every descendant are dropped by the
/// filter has nothing to upload, even when the walk would reach the
/// leaves. The caller short-circuits the same way as the single-file
/// case.
#[test]
fn has_permitted_entry_directory_everything_skipped_returns_false() {
	let dir = tempdir().unwrap();
	let outer = dir.path().join("outer");
	fs::create_dir_all(outer.join("nested")).unwrap();
	fs::write(outer.join("a.txt"), b"a").unwrap();
	fs::write(outer.join("nested").join("b.txt"), b"b").unwrap();

	let skip = |name: &Path| {
		let s = name.to_string_lossy().replace('\\', "/");
		s == "outer" || s.starts_with("outer/")
	};
	assert!(
		!has_permitted_entry(&outer, Path::new("outer"), &skip).unwrap(),
		"a directory the filter drops at every level has no permitted entry"
	);
}

/// A directory whose root the filter drops but that holds a permitted
/// deeper file must still report `true`. The walk descends through the
/// dropped root so a safe entry below it is still recognised, the same
/// way the packer still records it.
#[test]
fn has_permitted_entry_directory_root_skipped_descendant_permitted() {
	let dir = tempdir().unwrap();
	let outer = dir.path().join("outer");
	fs::create_dir_all(outer.join("nested")).unwrap();
	fs::write(outer.join("nested").join("ok.txt"), b"ok").unwrap();
	fs::write(outer.join("nested").join("bad.txt"), b"bad").unwrap();

	let skip = |name: &Path| {
		let s = name.to_string_lossy().replace('\\', "/");
		s == "outer" || s == "outer/nested" || s == "outer/nested/bad.txt"
	};
	assert!(
		has_permitted_entry(&outer, Path::new("outer"), &skip).unwrap(),
		"a deeper permitted file under a skipped root must report true"
	);
}

/// An empty directory with a non-empty entry name and a filter that
/// accepts the root entry is permitted: the packer would record the
/// root directory header so a file replaced by an empty directory still
/// becomes a directory in the container (#1985).
#[test]
fn has_permitted_entry_empty_directory_root_accepted() {
	let dir = tempdir().unwrap();
	let empty = dir.path().join("d");
	fs::create_dir(&empty).unwrap();
	let skip = |_name: &Path| false;
	assert!(
		has_permitted_entry(&empty, Path::new("d"), &skip).unwrap(),
		"an empty directory whose root entry the filter accepts has one permitted entry (the root)"
	);
}

/// An empty directory with an empty entry name and a filter that
/// accepts the empty name is permitted: the empty `entry_name`
/// represents the destination directory itself, and the caller
/// `sync_to_container` still has to run its `mkdir -p` for the
/// initial sync to leave the container in a useful state. A
/// caller that already decided the directory should not be
/// skipped is free to walk the destination instead.
#[test]
fn has_permitted_entry_empty_directory_empty_entry_name_filter_accepts_returns_true() {
	let dir = tempdir().unwrap();
	let empty = dir.path().join("d");
	fs::create_dir(&empty).unwrap();
	let skip = |_name: &Path| false;
	assert!(
		has_permitted_entry(&empty, Path::new(""), &skip).unwrap(),
		"an empty directory with an empty entry name the filter accepts has a permitted entry (the destination itself)"
	);
}

/// An empty directory with an empty entry name and a filter that
/// rejects the empty name has nothing to upload: the empty
/// `entry_name` represents the destination directory and the
/// filter says it is unsyncable, and the directory is empty so
/// there are no descendants to escape through. The caller
/// short-circuits the same way as the single-file skip case.
#[test]
fn has_permitted_entry_empty_directory_empty_entry_name_filter_rejects_returns_false() {
	let dir = tempdir().unwrap();
	let empty = dir.path().join("d");
	fs::create_dir(&empty).unwrap();
	let skip = |name: &Path| name.as_os_str().is_empty();
	assert!(
		!has_permitted_entry(&empty, Path::new(""), &skip).unwrap(),
		"an empty directory whose filter rejects the empty entry name has no permitted entry"
	);
}

// --- per-entry filter: root and single files ----------------------------

/// A directory upload with a non-empty entry name whose filter rejects
/// exactly the root name: the root directory header is dropped (the
/// packer still walks through it), its descendants are recorded
/// under their re-rooted paths, and the recorded `SentEntry` list
/// matches the tar contents. The recorder and the archive bytes
/// cannot drift, so a sabotage that drops an entry from the recorder
/// but keeps it in the tar (or the reverse) is caught.
#[test]
fn sync_tar_filter_drops_the_root_directory_keeps_descendants() {
	let dir = tempdir().unwrap();
	let outer = dir.path().join("outer");
	fs::create_dir(&outer).unwrap();
	fs::write(outer.join("a.txt"), b"a").unwrap();
	fs::create_dir(outer.join("sub")).unwrap();
	fs::write(outer.join("sub/b.txt"), b"b").unwrap();

	let skip = |name: &Path| name == Path::new("outer");
	let (bytes, sent) =
		sync_to_gz_with_sent(&outer, Path::new("outer"), &skip).expect("pack succeeds");

	let entries = tar_entries(&bytes);
	assert_eq!(
		entries,
		vec![
			("outer/a.txt".into(), "file"),
			("outer/sub".into(), "dir"),
			("outer/sub/b.txt".into(), "file"),
		],
		"filter on the root must drop the root directory entry and keep the descendants, got {entries:?}"
	);

	let sent_names: Vec<String> = sent.iter().map(|e| e.path().replace('\\', "/")).collect();
	assert_eq!(
		sent_names,
		vec![
			"outer/a.txt".to_string(),
			"outer/sub".to_string(),
			"outer/sub/b.txt".to_string(),
		],
		"recorded SentEntry list must match the tar contents, got {sent_names:?}"
	);
}

/// A single-file upload whose filter rejects its entry name records
/// nothing: the tar is a valid empty gzipped stream and the recorded
/// `SentEntry` list is empty. The verification step would then have
/// nothing to confirm against and the short-circuit in
/// `sync_to_container` would have already returned `Ok(())` before
/// the PUT ran.
#[test]
fn sync_tar_filter_drops_a_single_file() {
	let dir = tempdir().unwrap();
	let file = dir.path().join("solo.txt");
	fs::write(&file, b"only").unwrap();

	let skip = |name: &Path| name == Path::new("solo.txt");
	let (bytes, sent) =
		sync_to_gz_with_sent(&file, Path::new("solo.txt"), &skip).expect("pack succeeds");

	// The tar is still a valid gzipped stream, just with no entries.
	assert_eq!(
		&bytes[..2],
		&[0x1f, 0x8b],
		"tar must still be a valid gzip stream"
	);
	let entries = tar_entries(&bytes);
	assert!(
		entries.is_empty(),
		"the only entry the filter dropped must not appear in the tar, got {entries:?}"
	);
	assert!(
		sent.is_empty(),
		"the only entry the filter dropped must not be recorded, got {sent:?}"
	);
}

// --- entry_is_unsyncable ------------------------------------------------

/// An entry whose component is a non-UTF-8 byte (`<0xff>`) is
/// unsyncable: the lossy form of `container_rel` substitutes the
/// replacement character, so this name would collide with the
/// other bytes that map to the same character. The write-back
/// filter inside `sync_to_container` drops the entry, and the gate
/// in `maybe_sync` short-circuits the whole sync for a non-UTF-8
/// top-level path so the lossy mapping never runs.
#[cfg(unix)]
#[test]
fn entry_is_unsyncable_true_for_non_utf8_component() {
	use std::ffi::OsStr;
	use std::os::unix::ffi::OsStrExt;
	let p = Path::new(OsStr::from_bytes(b"\xff")).join("keep.txt");
	assert!(
		entry_is_unsyncable(&p),
		"a path whose component is non-UTF-8 must be reported unsyncable"
	);
}

/// A plain ASCII path is syncable: the filter inside
/// `sync_to_container` runs the write-back check, not the UTF-8
/// check, and the gate in `maybe_sync` lets the dispatch reach it.
#[test]
fn entry_is_unsyncable_false_for_utf8_path() {
	let p = Path::new("ok").join("keep.txt");
	assert!(
		!entry_is_unsyncable(&p),
		"a UTF-8 path must not be reported unsyncable"
	);
}
