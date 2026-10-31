//! The open of a single file named on the command line does not follow a link
//! put under the name after its type was looked at.

use super::open_without_following;

#[test]
fn an_ordinary_file_is_opened() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("file");
	std::fs::write(&path, b"x").unwrap();
	assert!(open_without_following(&path).is_ok());
}

#[cfg(unix)]
#[test]
fn a_link_is_refused_on_unix() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("target");
	std::fs::write(&target, b"x").unwrap();
	let link = dir.path().join("link");
	std::os::unix::fs::symlink(&target, &link).unwrap();
	assert!(open_without_following(&link).is_err());
}

/// Creating a link on Windows needs a privilege some accounts lack. Where it is
/// missing there is nothing to refuse, and the test says nothing.
#[cfg(windows)]
#[test]
fn a_link_is_refused_on_windows() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("target");
	std::fs::write(&target, b"x").unwrap();
	let link = dir.path().join("link");
	if std::os::windows::fs::symlink_file(&target, &link).is_err() {
		return;
	}
	assert!(open_without_following(&link).is_err());
}
