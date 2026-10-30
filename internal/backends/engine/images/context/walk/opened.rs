//! Walk build inputs while keeping directory and file identities pinned.

use std::fs::{File, Metadata};
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

/// An entry whose content is read without reopening its host path.
pub(in crate::backends::engine) enum Entry {
	/// A regular file opened without following its path again.
	File(File),
	/// Directory metadata read from its open descriptor.
	Directory(Metadata),
	/// Link text, never the bytes at the link target.
	Symlink(PathBuf),
}

/// Visit sorted entries, pruning before descent and never following child links.
/// The explicitly supplied root may resolve through links; descendants may not.
pub(in crate::backends::engine) fn visit_dir_skipping(
	root: &Path,
	skip_dir: impl Fn(&Path) -> bool,
	include_entry: impl Fn(&Path) -> bool,
	visit: impl FnMut(&Path, Entry) -> io::Result<()>,
) -> io::Result<()> {
	walk_from(root, true, skip_dir, include_entry, visit)
}

/// As [`visit_dir_skipping`], but the root is not followed either: a root that
/// is a link is an error. For a root that was chosen by something other than
/// the user, such as a path that arrived with a file-system event, where the
/// link is whatever replaced the directory after it was looked at.
#[cfg(feature = "watch")]
pub(in crate::backends::engine) fn visit_dir_pinned(
	root: &Path,
	skip_dir: impl Fn(&Path) -> bool,
	include_entry: impl Fn(&Path) -> bool,
	visit: impl FnMut(&Path, Entry) -> io::Result<()>,
) -> io::Result<()> {
	walk_from(root, false, skip_dir, include_entry, visit)
}

fn walk_from(
	root: &Path,
	follow_root: bool,
	skip_dir: impl Fn(&Path) -> bool,
	include_entry: impl Fn(&Path) -> bool,
	mut visit: impl FnMut(&Path, Entry) -> io::Result<()>,
) -> io::Result<()> {
	#[cfg(unix)]
	{
		unix::walk(root, follow_root, &skip_dir, &include_entry, &mut visit)
	}
	#[cfg(windows)]
	{
		windows::walk(root, follow_root, &skip_dir, &include_entry, &mut visit)
	}
	#[cfg(not(any(unix, windows)))]
	{
		let _ = (root, follow_root, skip_dir, include_entry, visit);
		Err(io::Error::new(
			io::ErrorKind::Unsupported,
			"secure build context traversal is unavailable",
		))
	}
}

/// A socket, pipe or device in the context. It is refused rather than sent, and
/// the message names it so it can be added to `.dockerignore`.
#[cfg(unix)]
fn special_file(path: &Path) -> io::Error {
	io::Error::other(format!(
		"build context entry {} is a socket, pipe or device; remove it or list it in .dockerignore",
		path.display()
	))
}

fn changed_type() -> io::Error {
	io::Error::other("build context entry changed type or is not a regular file")
}
