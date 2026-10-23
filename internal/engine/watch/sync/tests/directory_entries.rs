use super::*;

// --- directory entries --------------------------------------------------

/// An empty directory with a non-empty `entry_name` packs exactly one entry:
/// the directory itself. A file replaced by an empty directory must become a
/// directory in the container, not silently disappear (#1985).
#[test]
fn sync_tar_empty_directory_emits_one_directory_entry() {
	let dir = tempdir().unwrap();
	let empty = dir.path().join("d");
	fs::create_dir(&empty).unwrap();
	let bytes = sync_to_gz(&empty, Path::new("d")).unwrap();
	let entries = tar_entries(&bytes);
	assert_eq!(
		entries,
		vec![("d".into(), "dir")],
		"expected exactly one directory entry named `d`, got {entries:?}"
	);
}

/// A directory with one file packs the directory first, then its descendants,
/// in that order. An empty subpath the watcher reported must not turn the
/// archive into just the file with no parent directory.
#[test]
fn sync_tar_directory_emits_root_entry_then_descendants() {
	let dir = tempdir().unwrap();
	let d = dir.path().join("d");
	fs::create_dir(&d).unwrap();
	fs::write(d.join("a.txt"), b"a").unwrap();
	let bytes = sync_to_gz(&d, Path::new("d")).unwrap();
	let entries = tar_entries(&bytes);
	assert_eq!(
		entries,
		vec![("d".into(), "dir"), ("d/a.txt".into(), "file")],
		"expected dir then file, got {entries:?}"
	);
}

/// An empty `entry_name` (the initial sync of a rule root into its target)
/// does not add a root wrapper, matching the pre-#1985 behaviour for the
/// initial sync call: only the descendants are recorded.
#[test]
fn sync_tar_directory_with_empty_entry_name_emits_no_root_entry() {
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("a.txt"), b"a").unwrap();
	fs::create_dir(dir.path().join("sub")).unwrap();
	fs::write(dir.path().join("sub/b.txt"), b"b").unwrap();
	let bytes = sync_to_gz(dir.path(), Path::new("")).unwrap();
	let mut names = tar_entry_paths(&bytes);
	names.sort();
	assert_eq!(
		names,
		vec!["a.txt", "sub/b.txt"],
		"empty entry_name must not add a root entry"
	);
	// And there must be no root directory entry either.
	let kinds = tar_entries(&bytes);
	for (path, kind) in &kinds {
		if path.is_empty() {
			panic!("empty entry_name produced an entry with empty path: {kinds:?}");
		}
		if path == "." && *kind == "dir" {
			panic!("empty entry_name produced a `.` directory entry: {kinds:?}");
		}
	}
}
