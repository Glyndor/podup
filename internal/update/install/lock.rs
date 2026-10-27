//! Per-backup exclusion across updater processes.

use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::ComposeError;

/// Keep the returned handle alive until backup cleanup or rollback finishes.
/// Never unlink the sidecar: contenders must keep opening the same inode.
pub(super) fn acquire(target: &Path) -> crate::Result<File> {
	// Match the backup's naming scheme so targets sharing `.old` also share a lock.
	let path = target.with_extension("old.lock");
	let mut options = OpenOptions::new();
	options.read(true).write(true).create(true).truncate(false);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
	}
	#[cfg(windows)]
	{
		use std::os::windows::fs::OpenOptionsExt;
		// No read/write/delete sharing; closing the handle releases exclusivity.
		options.share_mode(0);
	}
	let file = options.open(&path).map_err(|e| {
		ComposeError::Update(format!("cannot open update lock {}: {e}", path.display()))
	})?;
	#[cfg(unix)]
	try_lock(&file).map_err(|e| {
		ComposeError::Update(format!(
			"cannot acquire update lock {} (another update may be running): {e}",
			path.display()
		))
	})?;
	Ok(file)
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn try_lock(file: &File) -> std::io::Result<()> {
	use std::os::fd::AsRawFd;
	// SAFETY: the descriptor belongs to `file` and stays open throughout the call.
	if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
		Ok(())
	} else {
		Err(std::io::Error::last_os_error())
	}
}
