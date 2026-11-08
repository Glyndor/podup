//! The modes an extraction applies, sanitized before they reach the inode.

use std::fs::File;
use std::io;

use rustix::fs::{self, Mode};

// mode_t is narrower than the tar header's u32 on some BSD targets.
#[allow(clippy::unnecessary_cast)]
pub(super) fn masked_mode(mode: u32) -> Mode {
	Mode::from_raw_mode((mode & 0o755) as fs::RawMode)
}

/// Sanitize modes on an open inode; no path lookup occurs during chmod.
pub(crate) fn set_mode(file: &File, mode: u32) -> io::Result<()> {
	Ok(fs::fchmod(file, masked_mode(mode))?)
}
