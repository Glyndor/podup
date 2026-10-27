//! Descriptor invariants independent of the container transport.

use std::ffi::OsStr;
use std::os::unix::fs::symlink;

use super::Directory;

#[cfg(unix)]
#[test]
fn child_open_refuses_a_replacement_symlink() {
	let tmp = tempfile::tempdir().unwrap();
	let outside = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	symlink(outside.path(), tmp.path().join("child")).unwrap();
	assert!(root.child(OsStr::new("child"), false).is_err());
	assert!(root.child(OsStr::new("child"), true).is_err());
	assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn flatten_uses_the_opened_wrapper_after_its_name_changes() {
	let tmp = tempfile::tempdir().unwrap();
	let dst = tmp.path().join("dest");
	let outside = tmp.path().join("outside");
	let moved = tmp.path().join("original");
	std::fs::create_dir_all(dst.join("wrapper")).unwrap();
	std::fs::create_dir(&outside).unwrap();
	std::fs::write(dst.join("wrapper/copied"), b"copied").unwrap();
	std::fs::write(outside.join("private"), b"private").unwrap();
	let root = Directory::open_host(&dst).unwrap();
	// The wrapper is already open when the namespace changes. Renames must
	// use that descriptor; removing the replacement link as a dir must fail.
	let result = root.flatten_after_open(|| {
		std::fs::rename(dst.join("wrapper"), &moved).unwrap();
		symlink(&outside, dst.join("wrapper")).unwrap();
	});
	assert!(result.is_err());
	assert_eq!(std::fs::read(outside.join("private")).unwrap(), b"private");
	assert_eq!(std::fs::read(dst.join("copied")).unwrap(), b"copied");
	assert!(!dst.join("private").exists());
}

#[cfg(unix)]
#[test]
fn file_creation_replaces_a_symlink_without_changing_its_target() {
	use std::io::Write;
	let tmp = tempfile::tempdir().unwrap();
	let outside = tmp.path().join("outside");
	std::fs::write(&outside, b"private").unwrap();
	symlink(&outside, tmp.path().join("file")).unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	root.file(OsStr::new("file"))
		.unwrap()
		.write_all(b"copied")
		.unwrap();
	assert_eq!(std::fs::read(&outside).unwrap(), b"private");
	assert_eq!(std::fs::read(tmp.path().join("file")).unwrap(), b"copied");
}

#[cfg(unix)]
#[test]
fn directory_chmod_uses_the_pinned_inode_without_read_permission() {
	use std::os::unix::fs::PermissionsExt;
	let tmp = tempfile::tempdir().unwrap();
	let path = tmp.path().join("dir");
	std::fs::create_dir(&path).unwrap();
	let pinned = Directory::open_host(&path).unwrap();
	std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o111)).unwrap();
	pinned.chmod(0o755).unwrap();
	assert_eq!(
		std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
		0o755
	);
}
