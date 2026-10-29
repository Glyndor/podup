//! A tree is packed through the descriptors the walk opened.
//!
//! The packer never reopens the path an entry was found at: the bytes that
//! go out are the bytes of the file the walk had open. The two tests here
//! pin that from both ends — a link stays a link, a file carries its own
//! bytes, and a directory that is replaced while the archive is being
//! written does not redirect the children that land in it.

use super::*;
use std::io::Write;
use std::path::PathBuf;

/// A writer that replaces a directory with a link to another tree the first
/// time the archive is written to.
///
/// The swap lands while the packer holds the names the walk produced, which
/// is the window a packer that reopens those names is exposed to: the name
/// still names the entry, but it points somewhere else by the time the
/// entry is read.
struct SwapOnFirstWrite<W> {
	inner: W,
	swap: Option<(PathBuf, PathBuf)>,
}

impl<W: Write> SwapOnFirstWrite<W> {
	fn new(inner: W, swap: (PathBuf, PathBuf)) -> Self {
		Self {
			inner,
			swap: Some(swap),
		}
	}
}

impl<W: Write> Write for SwapOnFirstWrite<W> {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		if let Some((dir, target)) = self.swap.take() {
			let moved = dir.with_extension("moved");
			std::fs::rename(&dir, &moved)?;
			std::os::unix::fs::symlink(&target, &dir)?;
		}
		self.inner.write(buf)
	}

	fn flush(&mut self) -> std::io::Result<()> {
		self.inner.flush()
	}
}

/// The body of one entry of an archive, read inside the iteration: each
/// entry's data is a `Take` over the archive's reader, so reading after the
/// iteration would advance past the body.
fn entry_body(bytes: &[u8], path: &str) -> (tar::EntryType, Option<Vec<u8>>, Vec<u8>) {
	let mut kind = tar::EntryType::Directory;
	let mut link = None;
	let mut body = Vec::new();
	let mut archive = tar::Archive::new(bytes);
	for entry in archive.entries().expect("entries") {
		let mut entry = entry.expect("entry");
		if entry.path().expect("path").to_string_lossy() == path {
			kind = entry.header().entry_type();
			link = entry.link_name_bytes().map(|b| b.into_owned());
			entry.read_to_end(&mut body).expect("read entry");
		}
	}
	(kind, link, body)
}

/// A tree with a link inside it and a file that spans several blocks.
///
/// Without `-L` the link stays a link: the archive holds a symlink entry with
/// the target the link named, and the recorder holds the same target. The
/// file is compared against the bytes on disk, so a file that was packed from
/// somewhere else would show up here.
#[cfg(unix)]
#[test]
fn a_link_inside_a_tree_is_packed_as_a_link_and_files_carry_their_own_bytes() {
	let dir = tempfile::tempdir().expect("tempdir");
	let tree = dir.path().join("datos");
	std::fs::create_dir_all(tree.join("sub")).expect("mkdir");

	let payload: Vec<u8> = (0..(300 * 1024)).map(|i| (i % 251) as u8).collect();
	std::fs::write(tree.join("sub").join("credencial"), &payload).expect("write");

	let outside = dir.path().join("outside.txt");
	std::fs::write(&outside, b"OUTSIDE-MARKER").expect("write");
	std::os::unix::fs::symlink(&outside, tree.join("sub").join("enlace")).expect("symlink");

	let (bytes, recorded) = pack_with_recorder(&tree, false, None, false);

	// The recorder holds the link as a link, with the target it named.
	let link = recorded
		.iter()
		.find(|e| e.path == "datos/sub/enlace")
		.expect("the link must be recorded");
	assert_eq!(
		link.kind,
		crate::engine::copy::verify::SentKind::Link(
			outside.to_str().expect("utf-8 target").to_string()
		),
		"a link inside the tree is recorded as a link, not as the file it points at"
	);

	// The archive holds a symlink entry for it, never the target's bytes.
	let (kind, target, _) = entry_body(&bytes, "datos/sub/enlace");
	assert_eq!(kind, tar::EntryType::Symlink, "the link stays a link");
	assert_eq!(
		target.expect("the entry names its target").as_slice(),
		outside.to_str().expect("utf-8 target").as_bytes(),
		"the entry carries the target the link named"
	);

	// The file carries the bytes of the file that was opened.
	let (kind, _, body) = entry_body(&bytes, "datos/sub/credencial");
	assert_eq!(kind, tar::EntryType::Regular);
	assert_eq!(
		body, payload,
		"the archive must hold the bytes of the file that was opened"
	);
}

/// A directory that is replaced by a link while the archive is being written
/// does not redirect the children that land in it.
///
/// The tree is packed through the descriptors the walk opened, so the file
/// that goes out is the one the directory held when it was opened. A packer
/// that reopens the names the walk produced would read the other tree's file
/// and ship its bytes under the name of the first one.
#[cfg(unix)]
#[test]
fn replacing_a_visited_directory_with_a_link_does_not_redirect_its_children() {
	let dir = tempfile::tempdir().expect("tempdir");
	let outside = tempfile::tempdir().expect("tempdir");
	let tree = dir.path().join("datos");
	let sub = tree.join("sub");
	std::fs::create_dir_all(&sub).expect("mkdir");
	std::fs::write(sub.join("credencial"), b"INSIDE").expect("write");
	std::fs::write(outside.path().join("credencial"), b"OUTSIDE-MARKER").expect("write");

	let mut buf = Vec::new();
	let mut sent = Vec::new();
	{
		let mut tar = tar::Builder::new(SwapOnFirstWrite::new(
			&mut buf,
			(sub.clone(), outside.path().to_path_buf()),
		));
		super::super::pack_path(&tree, false, None, false, &mut tar, &mut sent).expect("pack_path");
	}

	let (kind, _, body) = entry_body(&buf, "datos/sub/credencial");
	assert_eq!(kind, tar::EntryType::Regular);
	assert_eq!(
		body, b"INSIDE",
		"the entry must carry the bytes of the file the directory held when it was opened"
	);
}
