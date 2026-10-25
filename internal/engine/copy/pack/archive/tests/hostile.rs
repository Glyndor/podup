//! Host trees written to hurt the packer: loops, special files and entries
//! that cannot be named. Each case must end, quickly, and say why.

use std::sync::mpsc;
use std::time::Duration;

/// Run `pack_path` on a thread and give it a budget, because the failure being
/// guarded against is a walk that never ends. A thread that outlives the
/// budget is left behind; the test has already failed by then.
fn pack_within_budget(
	src: &std::path::Path,
	follow_link: bool,
	contents: bool,
) -> crate::error::Result<Vec<u8>> {
	let src = src.to_path_buf();
	let (tx, rx) = mpsc::channel();
	std::thread::spawn(move || {
		let mut buf = Vec::new();
		let mut sent = Vec::new();
		let result = {
			let mut tar = tar::Builder::new(&mut buf);
			super::super::pack_path(&src, follow_link, None, contents, &mut tar, &mut sent)
		};
		let _ = tx.send(result.map(|()| buf));
	});
	rx.recv_timeout(Duration::from_secs(30))
		.expect("pack_path did not return within 30 seconds")
}

fn mkfifo(path: &std::path::Path) {
	let status = std::process::Command::new("mkfifo")
		.arg(path)
		.status()
		.expect("mkfifo");
	assert!(status.success());
}

#[test]
fn a_symlink_loop_under_follow_link_ends_with_a_bounded_archive() {
	let dir = tempfile::tempdir().expect("tempdir");
	let tree = dir.path().join("tree");
	std::fs::create_dir(&tree).expect("mkdir");
	std::fs::write(tree.join("f"), b"x").expect("write");
	std::os::unix::fs::symlink(".", tree.join("self")).expect("symlink");
	std::os::unix::fs::symlink("../tree", tree.join("up")).expect("symlink");
	// Ending is the property: a loop followed without end would never return
	// or would fill memory. Whether the walk errors or stops one level down
	// is its own business, so the result is not asserted, only the size.
	if let Ok(bytes) = pack_within_budget(&tree, true, false) {
		assert!(
			bytes.len() < 1 << 20,
			"{} bytes from a 1 byte tree",
			bytes.len()
		);
	}
}

#[test]
fn a_symlink_loop_without_follow_link_is_stored_as_links() {
	let dir = tempfile::tempdir().expect("tempdir");
	let tree = dir.path().join("tree");
	std::fs::create_dir(&tree).expect("mkdir");
	std::os::unix::fs::symlink(".", tree.join("self")).expect("symlink");
	let bytes = pack_within_budget(&tree, false, false).expect("pack");
	let mut archive = tar::Archive::new(&bytes[..]);
	let kinds: Vec<_> = archive
		.entries()
		.expect("entries")
		.map(|e| e.expect("entry").header().entry_type())
		.collect();
	// The link, then the wrapper directory: nothing was walked through the link.
	assert_eq!(kinds, [tar::EntryType::Symlink, tar::EntryType::Directory]);
}

#[test]
fn a_fifo_inside_a_tree_is_refused_instead_of_read() {
	let dir = tempfile::tempdir().expect("tempdir");
	let tree = dir.path().join("tree");
	std::fs::create_dir(&tree).expect("mkdir");
	mkfifo(&tree.join("pipe"));
	for follow in [false, true] {
		let err = pack_within_budget(&tree, follow, false).expect_err("a FIFO must stop the pack");
		assert!(
			err.to_string().contains("unverified kind"),
			"follow={follow}: {err}"
		);
	}
}

#[test]
fn a_socket_inside_a_tree_is_refused() {
	let dir = tempfile::tempdir().expect("tempdir");
	let tree = dir.path().join("tree");
	std::fs::create_dir(&tree).expect("mkdir");
	let _listener = std::os::unix::net::UnixListener::bind(tree.join("s")).expect("bind");
	let err = pack_within_budget(&tree, false, true).expect_err("a socket must stop the pack");
	assert!(err.to_string().contains("unverified kind"), "{err}");
}

#[test]
fn a_symlink_whose_target_is_not_utf8_is_refused() {
	use std::os::unix::ffi::OsStrExt;
	let dir = tempfile::tempdir().expect("tempdir");
	let tree = dir.path().join("tree");
	std::fs::create_dir(&tree).expect("mkdir");
	let target = std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe");
	std::os::unix::fs::symlink(target, tree.join("l")).expect("symlink");
	assert!(pack_within_budget(&tree, false, false).is_err());
}

#[test]
fn a_file_name_that_is_not_utf8_is_refused() {
	use std::os::unix::ffi::OsStrExt;
	let dir = tempfile::tempdir().expect("tempdir");
	let tree = dir.path().join("tree");
	std::fs::create_dir(&tree).expect("mkdir");
	// APFS refuses such a name outright; with no such file there is nothing
	// to pack, so the precondition decides, not the platform name.
	if std::fs::write(tree.join(std::ffi::OsStr::from_bytes(b"bad\xff")), b"x").is_err() {
		return;
	}
	assert!(pack_within_budget(&tree, false, false).is_err());
}
