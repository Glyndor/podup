//! The `-L` classification of a path: one open that decides the kind and hands
//! over the bytes, so that what is read is what was classified.

use std::fs::File;
use std::path::Path;

/// What a followed path turned out to be.
pub(super) enum Followed {
	/// A regular file, with the handle its kind was read from.
	File(File),
	/// A directory.
	Directory,
	/// Anything the destination cannot ask about: FIFO, device, socket.
	Other,
}

/// Follow every link in `path` and classify what is at the end.
///
/// The kind is looked up first, following links, and anything that is neither a
/// regular file nor a directory is refused without being opened: opening a
/// device can act on it even when the open does not block. A regular file is
/// then opened, and the descriptor is checked against what was looked up (see
/// [`open_regular`]), so a path swapped in between is not read.
pub(super) fn open_following(path: &Path) -> std::io::Result<Followed> {
	let metadata = std::fs::metadata(path)?;
	if metadata.is_dir() {
		return Ok(Followed::Directory);
	}
	if !metadata.is_file() {
		return Ok(Followed::Other);
	}
	Ok(match open_regular(path, &metadata)? {
		Some(file) => Followed::File(file),
		None => Followed::Other,
	})
}

/// Open `path`, which was looked up as the regular file `expected`, and give the
/// handle back only if it is still that file.
///
/// On Unix the open is non-blocking and never takes a controlling terminal, so
/// a FIFO put under the name after the lookup is not waited for; the descriptor
/// is then checked, and a different kind or a different device and inode is
/// refused as `None`.
fn open_regular(path: &Path, expected: &std::fs::Metadata) -> std::io::Result<Option<File>> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
		let file = std::fs::OpenOptions::new()
			.read(true)
			.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
			.open(path)?;
		let opened = file.metadata()?;
		let same =
			opened.is_file() && opened.dev() == expected.dev() && opened.ino() == expected.ino();
		Ok(same.then_some(file))
	}
	#[cfg(not(unix))]
	{
		let _ = expected;
		Ok(Some(File::open(path)?))
	}
}

#[cfg(all(test, unix))]
#[path = "follow_tests.rs"]
mod tests;
