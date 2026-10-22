use super::*;

/// Existing regular file at the source path: the helper's decision must be
/// `Present`, the file must keep its bytes, and nothing else must be
/// created at or beside it.
#[test]
fn ensure_bind_source_existing_file_is_present() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("probe");
	std::fs::write(&path, b"keep-me").unwrap();
	let outcome = ensure_bind_source(path.to_str().unwrap());
	assert_eq!(outcome, BindSource::Present);
	let meta = std::fs::symlink_metadata(&path).unwrap();
	assert!(meta.is_file(), "source must remain a regular file");
	assert_eq!(std::fs::read(&path).unwrap(), b"keep-me");
}

/// Existing directory at the source path: the helper's decision must be
/// `Present`, and the directory's mtime must be untouched (no second
/// `create_dir_all`, no chmod).
#[test]
fn ensure_bind_source_existing_directory_is_present() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("already-here");
	std::fs::create_dir(&path).unwrap();
	let original_meta = std::fs::symlink_metadata(&path).unwrap();
	let outcome = ensure_bind_source(path.to_str().unwrap());
	assert_eq!(outcome, BindSource::Present);
	let after_meta = std::fs::symlink_metadata(&path).unwrap();
	assert_eq!(
		original_meta.modified().unwrap(),
		after_meta.modified().unwrap(),
		"existing directory mtime must be untouched"
	);
}

/// Dangling symlink at the source path: `symlink_metadata` returns `Ok`
/// for it, so the helper's decision must be `Present`, the symlink itself
/// must stay, and the target must NOT be created.
#[cfg(unix)]
#[test]
fn ensure_bind_source_dangling_symlink_is_present() {
	use std::os::unix::fs::symlink;
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("link");
	let target = dir.path().join("never");
	symlink(&target, &link).unwrap();
	let outcome = ensure_bind_source(link.to_str().unwrap());
	assert_eq!(outcome, BindSource::Present);
	let meta = std::fs::symlink_metadata(&link).unwrap();
	assert!(meta.file_type().is_symlink(), "link must remain a symlink");
	assert_eq!(std::fs::read_link(&link).unwrap(), target);
	assert!(
		!target.exists(),
		"symlink target must not be created by ensure_bind_source"
	);
}

/// Missing path under an existing directory: the helper's decision must be
/// `Created`, and the path must now be a directory.
#[test]
fn ensure_bind_source_missing_path_is_created() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("fresh");
	assert!(!path.exists());
	let outcome = ensure_bind_source(path.to_str().unwrap());
	assert_eq!(outcome, BindSource::Created);
	assert!(
		path.is_dir(),
		"missing path should be created as a directory"
	);
}

/// Missing path whose immediate parent is a regular FILE: `create_dir_all`
/// must fail because the parent is not a directory. The helper's decision
/// must be `Failed(msg)`, the message must mention the path, and nothing
/// must have been created where the helper could not.
#[cfg(unix)]
#[test]
fn ensure_bind_source_failure_under_file_is_failed() {
	let dir = tempfile::tempdir().unwrap();
	let blocker = dir.path().join("blocker");
	std::fs::write(&blocker, b"i-am-a-file").unwrap();
	let target = blocker.join("nested");
	let target_str = target.to_str().unwrap().to_string();
	assert!(!target.exists());
	let outcome = ensure_bind_source(&target_str);
	match outcome {
		BindSource::Failed(msg) => {
			assert!(
				msg.contains(&target_str),
				"warning text should mention the failing path, got: {msg}"
			);
		}
		other => panic!("expected Failed, got {other:?}"),
	}
	assert!(!target.exists(), "nothing must be created on failure");
	assert_eq!(
		std::fs::read(&blocker).unwrap(),
		b"i-am-a-file",
		"the blocking file must remain unchanged"
	);
}
