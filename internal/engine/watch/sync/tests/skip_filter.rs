use super::*;

// --- per-entry skip filter ----------------------------------------------

/// A filter that drops both `outer/nested` (the directory) and `outer/nested/f`
/// (the file) must keep the root directory `outer` and the sibling file
/// `outer/ok.txt`, but record nothing under `outer/nested` because the
/// directory itself is dropped (#1985). The walker still descends into the
/// dropped directory so the filter can decide on each entry independently.
#[test]
fn sync_tar_filter_drops_a_directory_and_its_contents() {
	let dir = tempdir().unwrap();
	let outer = dir.path().join("outer");
	let nested = outer.join("nested");
	fs::create_dir_all(&nested).unwrap();
	fs::write(nested.join("f"), b"nested content").unwrap();
	fs::write(outer.join("ok.txt"), b"ok").unwrap();

	let skip = |name: &Path| {
		let s = name.to_string_lossy().replace('\\', "/");
		s == "outer/nested" || s == "outer/nested/f"
	};
	let bytes = sync_to_gz_with(&outer, Path::new("outer"), &skip).unwrap();
	let entries = tar_entries(&bytes);
	assert_eq!(
		entries,
		vec![("outer".into(), "dir"), ("outer/ok.txt".into(), "file")],
		"expected root dir + sibling file only, got {entries:?}"
	);
}

/// A filter that drops the `outer/nested` directory but not `outer/nested/f`
/// keeps `outer`, drops the directory header, and records `outer/nested/f` as
/// a file at the deeper path. The walker must descend into a directory the
/// filter dropped so a safe deeper mount below a loop-causing directory is
/// still copied.
#[test]
fn sync_tar_filter_drops_the_directory_but_keeps_descendants() {
	let dir = tempdir().unwrap();
	let outer = dir.path().join("outer");
	let nested = outer.join("nested");
	fs::create_dir_all(&nested).unwrap();
	fs::write(nested.join("f"), b"nested content").unwrap();
	fs::write(outer.join("ok.txt"), b"ok").unwrap();

	let skip = |name: &Path| {
		let s = name.to_string_lossy().replace('\\', "/");
		s == "outer/nested"
	};
	let bytes = sync_to_gz_with(&outer, Path::new("outer"), &skip).unwrap();
	let entries = tar_entries(&bytes);
	// The directory header is dropped; the descendant file is still recorded.
	let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
	assert!(
		names.contains(&"outer/nested/f"),
		"nested file must still be recorded, got {entries:?}"
	);
	assert!(
		!names.contains(&"outer/nested"),
		"nested directory header must be dropped, got {entries:?}"
	);
	assert!(
		names.contains(&"outer"),
		"outer root must still be recorded, got {entries:?}"
	);
	assert!(
		names.contains(&"outer/ok.txt"),
		"sibling file must still be recorded, got {entries:?}"
	);
}
