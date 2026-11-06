//! Creating the entries an archive carries, inside a pinned directory.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::Path;

use rustix::fs::{self, AtFlags, Mode, OFlags};
use rustix::io::Errno;

impl super::Directory {
	/// Create a fresh regular file; never truncate a symlink or shared inode.
	pub(crate) fn file(&self, name: &OsStr) -> io::Result<File> {
		let open = || {
			fs::openat(
				&self.0,
				name,
				OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
				Mode::from_raw_mode(0o666),
			)
		};
		match open() {
			Ok(fd) => Ok(fd.into()),
			Err(Errno::EXIST) => {
				self.unlink(name)?;
				Ok(open()?.into())
			}
			Err(e) => Err(e.into()),
		}
	}

	fn unlink(&self, name: &OsStr) -> io::Result<()> {
		match fs::unlinkat(&self.0, name, AtFlags::empty()) {
			Ok(()) | Err(Errno::NOENT) => Ok(()),
			Err(e) => Err(e.into()),
		}
	}

	/// Store a symlink as data; it will never be traversed by later operations.
	pub(crate) fn symlink(&self, name: &OsStr, target: &Path) -> io::Result<()> {
		match fs::symlinkat(target, &self.0, name) {
			Ok(()) => Ok(()),
			Err(Errno::EXIST) => {
				self.unlink(name)?;
				Ok(fs::symlinkat(target, &self.0, name)?)
			}
			Err(e) => Err(e.into()),
		}
	}

	/// Preserve a link's timestamp without following a replacement symlink.
	pub(crate) fn symlink_time(&self, name: &OsStr, mtime: u64) -> io::Result<()> {
		let time = fs::Timespec {
			tv_sec: mtime.max(1) as i64,
			tv_nsec: 0,
		};
		Ok(fs::utimensat(
			&self.0,
			name,
			&fs::Timestamps {
				last_access: time,
				last_modification: time,
			},
			AtFlags::SYMLINK_NOFOLLOW,
		)?)
	}
}
