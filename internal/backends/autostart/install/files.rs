//! How the unit files are written: a private directory, and units with an
//! explicit mode.
//!
//! `create_dir_all` and `fs::write` take their permissions from the process
//! umask, so a permissive umask leaves the unit directory and the units in it
//! writable by any local user. A unit another user can rewrite is a command
//! that user can run at boot, so the modes are set here rather than inherited.

use std::io::Write;
use std::path::Path;

/// The mode a unit directory gets: private to its owner.
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;

/// The mode a unit file gets: readable by its owner only. systemd reads user
/// units as the owning user, so nothing else needs it.
#[cfg(unix)]
const FILE_MODE: u32 = 0o600;

/// Create `dir` when it is missing, with a private mode.
///
/// An existing directory keeps the mode it has: it may be one the operator
/// arranged on purpose, and every unit inside it carries its own mode.
pub(in super::super) fn private_dir(dir: &Path) -> std::io::Result<()> {
	let mut builder = std::fs::DirBuilder::new();
	builder.recursive(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::DirBuilderExt;
		builder.mode(DIR_MODE);
	}
	builder.create(dir)
}

/// Write `contents` to `path` with an explicit mode.
///
/// The mode is applied when the file is created and again on a file that
/// already existed, so a unit written under a permissive umask does not stay
/// writable by another local user after a re-install.
pub(in super::super) fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
	let mut options = std::fs::OpenOptions::new();
	options.write(true).create(true).truncate(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(FILE_MODE);
	}
	let mut file = options.open(path)?;
	file.write_all(contents)?;
	file.sync_all()?;
	drop(file);
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		std::fs::set_permissions(path, std::fs::Permissions::from_mode(FILE_MODE))?;
	}
	Ok(())
}
