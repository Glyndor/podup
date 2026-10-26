use super::*;

#[test]
fn sync_tar_single_file() {
	let dir = tempdir().unwrap();
	let file = dir.path().join("hello.txt");
	fs::write(&file, b"hello world").unwrap();
	let bytes = sync_to_gz(&file, Path::new("hello.txt")).unwrap();
	// gzip magic bytes
	assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	assert_eq!(tar_entry_paths(&bytes), vec!["hello.txt"]);
}

#[test]
fn sync_tar_single_file_renamed_entry() {
	// A single-file rule whose target renames the file: the entry must be
	// stored under the supplied (target) name, not the source basename.
	let dir = tempdir().unwrap();
	let file = dir.path().join("settings.yml");
	fs::write(&file, b"k: v").unwrap();
	let bytes = sync_to_gz(&file, Path::new("config.yml")).unwrap();
	assert_eq!(tar_entry_paths(&bytes), vec!["config.yml"]);
}

#[test]
fn sync_tar_subpath_entry_preserved() {
	// A directory rule where a nested file changed: re-rooting under the
	// supplied entry name must preserve the subdirectory.
	let dir = tempdir().unwrap();
	let file = dir.path().join("b.txt");
	fs::write(&file, b"file b").unwrap();
	let bytes = sync_to_gz(&file, Path::new("sub/b.txt")).unwrap();
	assert_eq!(tar_entry_paths(&bytes), vec!["sub/b.txt"]);
}

#[test]
fn sync_tar_directory() {
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("a.txt"), b"file a").unwrap();
	fs::create_dir(dir.path().join("sub")).unwrap();
	fs::write(dir.path().join("sub/b.txt"), b"file b").unwrap();
	let bytes = sync_to_gz(dir.path(), Path::new("dst")).unwrap();
	assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	// Every descendant is re-rooted under the supplied entry name, layout kept.
	let mut names = tar_entry_paths(&bytes);
	names.sort();
	assert_eq!(names, vec!["dst/a.txt", "dst/sub/b.txt"]);
}

#[test]
fn sync_tar_path_with_no_file_name() {
	// A path that has no file_name (e.g. root "/"): tar should be empty but valid.
	let dir = tempdir().unwrap();
	// Empty directory, so no entries other than root
	let bytes = sync_to_gz(dir.path(), Path::new(".")).unwrap();
	assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
}

#[test]
fn sync_tar_missing_source_is_a_sync_error_not_a_build_error() {
	// The watch sync promises the user a sync failure reads as a sync
	// failure. `docs/commands.md` says the warning line is the only signal
	// a long-running `watch` leaves open, so a category swap to `build`
	// silently drops the original context. The original bug was this:
	// `build_sync_tar` mapped the tar packer's io error to `Build`, which
	// `Display` rendered as `build error: ...`.
	let missing = Path::new("/nonexistent-watch-source-xyz");
	let err = sync_to_gz(missing, Path::new("x")).unwrap_err();
	let msg = err.to_string();
	assert!(
		msg.contains("watch error"),
		"missing-source must be a watch/sync error, got: {msg:?}"
	);
	assert!(
		!msg.contains("build error"),
		"missing-source must not be a build error: {msg:?}"
	);
}

#[test]
fn sync_tar_missing_directory_source_is_a_sync_error_not_a_build_error() {
	// Same classification as the single-file case: a missing directory
	// source for a directory rule must read as a sync failure, never a
	// build failure.
	let missing = Path::new("/nonexistent-watch-source-dir-xyz");
	let err = sync_to_gz(missing, Path::new("x")).unwrap_err();
	let msg = err.to_string();
	assert!(msg.contains("watch error"), "wrong category: {msg:?}");
	assert!(
		!msg.contains("build error"),
		"must not be a build error: {msg:?}"
	);
}
