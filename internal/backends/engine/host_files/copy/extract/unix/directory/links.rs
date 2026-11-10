//! Linking a regular file this extraction already created.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::Path;

use rustix::fs::{self, AtFlags, Mode, OFlags};
use rustix::io::Errno;

#[cfg(test)]
use super::before_reopen;
use super::file_identity;
use super::modes::masked_mode;
use super::state::ExtractionState;

impl super::Directory {
	/// Link a regular file created by this extraction within the pinned root.
	///
	/// A source that existed before the extraction is refused: its inode may
	/// have other names outside the destination, and the mode in the archive
	/// is never applied to an inode the extraction did not create.
	pub(crate) fn hardlink(
		&self,
		name: &OsStr,
		root: &Self,
		target: &Path,
		mode: Option<u32>,
		state: &ExtractionState,
	) -> io::Result<Option<File>> {
		let (source, leaf) = root.parent(target, false)?;
		let stat = fs::statat(&source.0, &leaf, AtFlags::SYMLINK_NOFOLLOW)?;
		if fs::FileType::from_raw_mode(stat.st_mode) != fs::FileType::RegularFile {
			return Err(io::Error::other(format!(
				"cp: hard link source is not a regular file: {}",
				target.display()
			)));
		}
		let identity = file_identity(&stat);
		if !state.files.contains(&identity) {
			return Err(io::Error::other(format!(
				"cp: hard link source was not created by this extraction: {}",
				target.display()
			)));
		}
		// No SYMLINK_FOLLOW: a leaf changed after stat can only link the
		// symlink itself, never its target.
		fs::linkat(&source.0, &leaf, &self.0, name, AtFlags::empty())?;
		let linked = fs::statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW)?;
		if fs::FileType::from_raw_mode(linked.st_mode) != fs::FileType::RegularFile
			|| file_identity(&linked) != identity
		{
			return Err(io::Error::other(format!(
				"cp: hard link source identity changed: {}",
				target.display()
			)));
		}
		if mode.is_none_or(|mode| linked.st_mode & 0o7777 == masked_mode(mode).bits()) {
			return Ok(None);
		}
		#[cfg(test)]
		before_reopen::run();
		let open = |access| {
			fs::openat(
				&self.0,
				name,
				access | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
				Mode::empty(),
			)
		};
		let fd = match open(OFlags::RDONLY) {
			Err(Errno::ACCESS) => open(OFlags::WRONLY)?,
			other => other?,
		};
		// The name was opened again, so what came back is checked, not assumed:
		// a regular file with another identity is a different inode, and the mode
		// is never applied to it.
		let opened = fs::fstat(&fd)?;
		if fs::FileType::from_raw_mode(opened.st_mode) != fs::FileType::RegularFile
			|| file_identity(&opened) != identity
		{
			return Err(io::Error::other(format!(
				"cp: hard link changed while its mode was being applied: {}",
				target.display()
			)));
		}
		Ok(Some(fd.into()))
	}
}
