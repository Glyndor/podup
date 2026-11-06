//! `cp -L` through links to directories: a link met while a linked directory is
//! being read is followed too, and a link that leads back is refused.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A writer that fails past a budget, so a walk that never ends stops with an
/// error instead of filling the memory of the machine that runs the suite.
struct Bounded {
	bytes: Vec<u8>,
}

impl Write for Bounded {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		if self.bytes.len() + buf.len() > 4 << 20 {
			return Err(io::Error::other("test budget exceeded"));
		}
		self.bytes.extend_from_slice(buf);
		Ok(buf.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

fn pack(src: &Path, follow_link: bool) -> crate::error::Result<Vec<u8>> {
	let src = src.to_path_buf();
	let (tx, rx) = std::sync::mpsc::channel();
	std::thread::spawn(move || {
		let mut sink = Bounded { bytes: Vec::new() };
		let result = {
			let mut tar = tar::Builder::new(&mut sink);
			let mut sent = Vec::new();
			super::super::pack_path(&src, follow_link, None, false, &mut tar, &mut sent)
		};
		let _ = tx.send(result.map(|()| sink.bytes));
	});
	rx.recv_timeout(Duration::from_secs(20))
		.expect("pack_path did not return within 20 seconds")
}

/// Entries of an archive: path, type and, for a regular file, its bytes.
fn read(bytes: &[u8]) -> Vec<(String, tar::EntryType, Vec<u8>)> {
	use std::io::Read;
	let mut archive = tar::Archive::new(bytes);
	let mut out = Vec::new();
	for entry in archive.entries().expect("entries") {
		let mut entry = entry.expect("entry");
		let mut body = Vec::new();
		entry.read_to_end(&mut body).expect("read");
		out.push((
			entry.path().expect("path").to_string_lossy().into_owned(),
			entry.header().entry_type(),
			body,
		));
	}
	out
}

/// `tree/a -> outside_a`, `outside_a/b -> outside_b`, data in `outside_b`:
/// two links to directories in a row, no loop.
fn chain() -> (tempfile::TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("tempdir");
	let root = dir.path();
	for d in ["tree", "outside_a", "outside_b"] {
		std::fs::create_dir(root.join(d)).expect("mkdir");
	}
	std::fs::write(root.join("outside_b/important.txt"), b"the data").expect("write");
	std::os::unix::fs::symlink(root.join("outside_a"), root.join("tree/a")).expect("symlink");
	std::os::unix::fs::symlink(root.join("outside_b"), root.join("outside_a/b")).expect("symlink");
	let tree = root.join("tree");
	(dir, tree)
}

#[test]
fn a_link_to_a_directory_met_inside_a_followed_link_is_followed_too() {
	let (_dir, tree) = chain();
	let entries = read(&pack(&tree, true).expect("pack"));
	let found = entries
		.iter()
		.find(|(name, _, _)| name == "tree/a/b/important.txt")
		.unwrap_or_else(|| panic!("the file behind the second link is missing: {entries:?}"));
	assert_eq!(found.1, tar::EntryType::Regular);
	assert_eq!(found.2, b"the data");
}

#[test]
fn the_same_chain_without_follow_link_stays_two_links() {
	let (_dir, tree) = chain();
	let entries = read(&pack(&tree, false).expect("pack"));
	let names: Vec<_> = entries.iter().map(|(n, t, _)| (n.as_str(), *t)).collect();
	assert_eq!(
		names,
		[
			("tree/a", tar::EntryType::Symlink),
			("tree", tar::EntryType::Directory)
		]
	);
}

/// Two links to one directory are not a loop: both are copied in full.
#[test]
fn two_links_to_the_same_directory_are_both_copied() {
	let dir = tempfile::tempdir().expect("tempdir");
	let root = dir.path();
	for d in ["tree", "shared"] {
		std::fs::create_dir(root.join(d)).expect("mkdir");
	}
	std::fs::write(root.join("shared/f"), b"x").expect("write");
	std::os::unix::fs::symlink(root.join("shared"), root.join("tree/x")).expect("symlink");
	std::os::unix::fs::symlink(root.join("shared"), root.join("tree/y")).expect("symlink");
	let entries = read(&pack(&root.join("tree"), true).expect("pack"));
	let names: Vec<_> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
	assert!(names.contains(&"tree/x/f"), "{names:?}");
	assert!(names.contains(&"tree/y/f"), "{names:?}");
}

/// A link that leads back to a directory being copied is refused, not
/// followed without end and not cut short in silence.
#[test]
fn a_link_that_leads_back_is_refused_with_an_error() {
	let dir = tempfile::tempdir().expect("tempdir");
	let root = dir.path();
	for d in ["tree", "outside"] {
		std::fs::create_dir(root.join(d)).expect("mkdir");
	}
	std::fs::write(root.join("outside/f"), b"x").expect("write");
	std::os::unix::fs::symlink(root.join("outside"), root.join("tree/a")).expect("symlink");
	std::os::unix::fs::symlink(root.join("tree"), root.join("outside/back")).expect("symlink");
	let err = pack(&root.join("tree"), true).expect_err("a loop must stop the copy");
	assert!(err.to_string().contains("loop"), "{err}");
}
