use std::os::unix::fs::{MetadataExt, PermissionsExt};

use super::super::super::hostile_support::{archive, dir, file, hardlink, symlink as tar_symlink};
use super::super::{extract, Directory};

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
	use super::super::super::hostile_support::raw;
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
	use super::super::super::hostile_support::raw;
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
	use super::super::super::hostile_support::raw;
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

/// The name of a hard link is opened again to give it its mode. If another
/// regular file has been put under that name by then, the mode must not be
/// applied to it: the file may have other names outside the destination.
#[cfg(unix)]
#[test]
fn a_hard_link_replaced_before_its_mode_is_applied_is_not_changed() {
	use super::super::super::hostile_support::raw;
	let tmp = tempfile::tempdir().unwrap();
	let dst = tmp.path().join("dest");
	std::fs::create_dir(&dst).unwrap();
	let outside = tmp.path().join("outside");
	std::fs::write(&outside, b"private").unwrap();
	std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o600)).unwrap();
	let root = Directory::open_host(&dst).unwrap();
	let linked = dst.join("b");
	let (swap_from, swap_to) = (outside.clone(), linked.clone());
	super::super::directory::before_reopen::set(move || {
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
