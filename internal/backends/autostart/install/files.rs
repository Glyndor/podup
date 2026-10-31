//! How the unit files are written: a private directory, and units published
//! atomically over a destination podup owns.
//!
//! `create_dir_all` and `fs::write` take their permissions from the process
//! umask, so a permissive umask leaves the unit directory and the units in it
//! writable by any local user. A unit another user can rewrite is a command
//! that user can run at boot, so the modes are set here rather than
//! inherited.
//!
//! The write itself goes through a temporary file and a rename, so a
//! destination that is a symbolic link is never written through and a unit is
//! never half written.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The mode a unit file gets: readable by its owner only. systemd reads user
/// units as the owning user, so nothing else needs it.
#[cfg(unix)]
const FILE_MODE: u32 = 0o600;

/// Create missing directories privately and refuse unsafe existing unit directories.
pub(in super::super) fn private_dir(dir: &Path) -> std::io::Result<()> {
	crate::backends::unit_directory::require_private(dir)
}

/// Refuse a destination that is not a regular file.
///
/// A symbolic link is refused rather than followed: the link points at a file
/// podup did not write, and writing through it would change that file
/// instead of the unit. Anything that is not a regular file (a directory, a
/// fifo, a device) is refused for the same reason.
pub(in super::super) fn check_destination(path: &Path) -> std::io::Result<()> {
	match std::fs::symlink_metadata(path) {
		Ok(meta) if meta.file_type().is_symlink() => Err(std::io::Error::new(
			std::io::ErrorKind::InvalidInput,
			format!(
				"refusing to write {}: it is a symbolic link",
				path.display()
			),
		)),
		Ok(meta) if !meta.file_type().is_file() => Err(std::io::Error::new(
			std::io::ErrorKind::InvalidInput,
			format!(
				"refusing to write {}: it is not a regular file",
				path.display()
			),
		)),
		Ok(_) => Ok(()),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(e) => Err(e),
	}
}

/// Replace `path` with `contents`, atomically and without following a link.
///
/// The text goes to a new file next to the target, created exclusively (so it
/// can never be a pre-planted link or an existing file) with the unit's mode
/// from the start, and is then renamed over the target. The rename replaces
/// the entry at the name in one step, so the unit is never half written and
/// the target never exists with looser permissions.
pub(in super::super) fn publish_unit(path: &Path, contents: &[u8]) -> std::io::Result<()> {
	check_destination(path)?;
	let dir = path.parent().unwrap_or_else(|| Path::new("."));
	let name = path
		.file_name()
		.map(|n| n.to_string_lossy().into_owned())
		.unwrap_or_default();
	let mut attempt = 0u32;
	let (tmp, mut file) = loop {
		let tmp: PathBuf = dir.join(format!(".{name}.{}.{attempt}.tmp", std::process::id()));
		let mut options = std::fs::OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(FILE_MODE);
		}
		match options.open(&tmp) {
			Ok(file) => break (tmp, file),
			Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < 16 => attempt += 1,
			Err(e) => return Err(e),
		}
	};
	let result = file
		.write_all(contents)
		.and_then(|()| file.sync_all())
		.and_then(|()| {
			drop(file);
			std::fs::rename(&tmp, path)
		});
	if result.is_err() {
		let _ = std::fs::remove_file(&tmp);
	}
	result
}
