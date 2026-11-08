//! Collapsing the single wrapper directory a `cp` of a directory carries.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;

use rustix::fs::{self, AtFlags, Mode};

use super::READ_DIRECTORY_FLAGS;

impl super::Directory {
	pub(super) fn names(&self) -> io::Result<Vec<OsString>> {
		let fd = fs::openat(&self.0, ".", READ_DIRECTORY_FLAGS, Mode::empty())?;
		let entries = fs::Dir::new(fd)?;
		let mut names = Vec::new();
		for entry in entries {
			let entry = entry?;
			let bytes = entry.file_name().to_bytes();
			if bytes != b"." && bytes != b".." {
				names.push(OsStr::from_bytes(bytes).to_owned());
			}
		}
		Ok(names)
	}

	/// Collapse the single wrapper through pinned source and target directories.
	pub(crate) fn flatten(&self) -> io::Result<Option<OsString>> {
		self.flatten_after_open(|| {})
	}

	pub(super) fn flatten_after_open(
		&self,
		after_open: impl FnOnce(),
	) -> io::Result<Option<OsString>> {
		let children = self.names()?;
		if children.len() != 1 {
			return Ok(None);
		}
		let name = &children[0];
		let wrapper = match self.child(name, false) {
			Ok(dir) => dir,
			Err(e) if matches!(e.raw_os_error(), Some(libc::ENOTDIR | libc::ELOOP)) => {
				return Ok(None)
			}
			Err(e) => return Err(e),
		};
		after_open();
		for child in wrapper.names()? {
			fs::renameat(&wrapper.0, &child, &self.0, &child)?;
		}
		// unlinkat removes the directory entry itself; it never follows a link.
		fs::unlinkat(&self.0, name, AtFlags::REMOVEDIR)?;
		Ok(Some(name.clone()))
	}
}
