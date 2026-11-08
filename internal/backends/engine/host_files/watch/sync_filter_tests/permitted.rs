use super::*;

// --- has_permitted_entry -------------------------------------------------

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
