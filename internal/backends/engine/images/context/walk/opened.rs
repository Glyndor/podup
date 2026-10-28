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
	mut visit: impl FnMut(&Path, Entry) -> io::Result<()>,
) -> io::Result<()> {
	#[cfg(unix)]
	{
		unix::walk(root, &skip_dir, &include_entry, &mut visit)
	}
	#[cfg(windows)]
	{
		windows::walk(root, &skip_dir, &include_entry, &mut visit)
	}
	#[cfg(not(any(unix, windows)))]
	{
		let _ = (root, skip_dir, include_entry, visit);
		Err(io::Error::new(
			io::ErrorKind::Unsupported,
			"secure build context traversal is unavailable",
		))
	}
}

fn changed_type() -> io::Error {
	io::Error::other("build context entry changed type or is not a regular file")
}
