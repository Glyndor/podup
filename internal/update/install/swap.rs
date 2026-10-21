use std::io::Write;
use std::path::Path;

use crate::ComposeError;

/// Write the new bytes to `tmp`, copy `target`'s permission bits (default 0755
/// on Unix when the target does not yet exist), and flush to disk.
pub(crate) fn write_temp(tmp: &Path, new_bytes: &[u8], target: &Path) -> crate::Result<()> {
	// Create the temp file private (0600) on Unix so the new binary's bytes are
	// never world-readable in a shared directory (e.g. /usr/local/bin) during the
	// window before the target's mode is applied. `File::create` honours the
	// process umask and could otherwise leave a 0644 file readable by other users.
	#[cfg(unix)]
	let mut f = {
		use std::os::unix::fs::OpenOptionsExt;
		// Remove any stale temp (e.g. from a crashed run); unlinking a symlink
		// removes the link itself and does not follow it.
		let _ = std::fs::remove_file(tmp);
		// `create_new` (O_EXCL) + O_NOFOLLOW: never follow or clobber a pre-planted
		// symlink in a shared/attacker-writable install directory, so the verified
		// bytes can only land in our own freshly created file.
		//
		// **Neither flag is reachable from a test, and that is a property of what
		// they guard rather than a gap.** The `remove_file` above already unlinks
		// any symlink planted beforehand (measured: after it, the link is gone
		// and its victim is untouched) so what is left for these flags is the
		// window *between* that unlink and this open. Closing a race is exactly
		// the thing an in-process test cannot enter. Mutations removing either
		// one survive the suite; the third property of this call, the 0600 mode,
		// is reachable and is pinned by
		// `write_temp_creates_the_file_private_to_this_user`.
		std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.custom_flags(libc::O_NOFOLLOW)
			.mode(0o600)
			.open(tmp)
			.map_err(|e| {
				ComposeError::Update(format!("cannot write update to {}: {e}", tmp.display()))
			})?
	};
	#[cfg(windows)]
	let mut f = {
		use std::os::windows::fs::OpenOptionsExt;
		// Mirror the Unix branch's invariants on Windows. `create_new(true)`
		// is the Windows analogue of `O_CREAT|O_EXCL`: after the unlink
		// below, the open fails if anything reappeared at `tmp` rather
		// than truncating it, so a
		// planted junction cannot redirect the verified bytes into a host
		// path the operator did not name. `custom_flags(0x0020_0000)` is
		// `FILE_FLAG_OPEN_REPARSE_POINT`: the open is given a handle to
		// the reparse point itself rather than the target it points at,
		// failing the call rather than silently traversing the link.
		// Together with the `remove_file` above (which unlinks a planted
		// symlink rather than following it) the verified bytes can only
		// land in a freshly created file at `tmp`. The constants are
		// intentionally inlined rather than imported from `windows-sys`
		// to keep this branch buildable on hosts that only have the
		// `Console` features wired up.
		let _ = std::fs::remove_file(tmp);
		std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.custom_flags(0x0020_0000)
			.open(tmp)
			.map_err(|e| {
				ComposeError::Update(format!("cannot write update to {}: {e}", tmp.display()))
			})?
	};
	#[cfg(not(any(unix, windows)))]
	let mut f = std::fs::File::create(tmp).map_err(|e| {
		ComposeError::Update(format!("cannot write update to {}: {e}", tmp.display()))
	})?;
	f.write_all(new_bytes).map_err(ComposeError::Io)?;
	f.flush().map_err(ComposeError::Io)?;

	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		// Copy the target's permission bits, but mask off setuid/setgid/sticky
		// (`& 0o777`): podup is an ordinary binary, and propagating a special bit
		// from a tampered target onto the freshly installed binary would be a
		// privilege-escalation footgun.
		let mode = std::fs::metadata(target)
			.map(|m| m.permissions().mode() & 0o777)
			.unwrap_or(0o755);
		std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(mode))
			.map_err(ComposeError::Io)?;
	}
	#[cfg(not(unix))]
	{
		let _ = target; // permissions are inherited on non-Unix.
	}

	f.sync_all().map_err(ComposeError::Io)?;
	Ok(())
}

/// Atomically move `tmp` onto `target`. Unix replaces the inode directly;
/// Windows renames the in-use file aside first.
#[cfg(not(windows))]
pub(crate) fn swap_into_place(tmp: &Path, target: &Path) -> crate::Result<()> {
	std::fs::rename(tmp, target).map_err(|e| super::rename_error(e, target))
}

#[cfg(windows)]
pub(crate) fn swap_into_place(tmp: &Path, target: &Path) -> crate::Result<()> {
	// A running .exe cannot be overwritten, but it can be renamed. Move it aside,
	// put the new binary in place, then best-effort delete the old one (it may
	// still be locked while running - if so, it is removed at the start of the
	// next updater run by `cleanup_stale_backup`).
	//
	// The L5 swap path
	// (`install_binary` → `move_target_aside` → `install_at`) means the target
	// may already be absent when this function is called: the caller has moved
	// it to `.old` so the self-test has a recoverable copy on disk. Track
	// whether *this* call created the backup so the cleanup at the end only
	// touches the file we created, and the rename error path only tries to
	// restore from a backup we own.
	let backup = target.with_extension("old");
	let target_existed = target.exists();
	if target_existed {
		// Drop any stale leftover before re-creating it. Only meaningful when
		// we are about to be the one to create a fresh `.old`.
		let _ = std::fs::remove_file(&backup);
		std::fs::rename(target, &backup).map_err(|e| super::rename_error(e, target))?;
	}
	if let Err(e) = std::fs::rename(tmp, target) {
		// Roll back so the user is not left without a binary, but only when
		// we created the backup in this call. A pre-existing `.old` (the L5
		// swap path) is the caller's responsibility to restore.
		if target_existed {
			let _ = std::fs::rename(&backup, target);
		}
		return Err(super::rename_error(e, target));
	}
	// Only clean up the backup if we created it in this call. A pre-existing
	// `.old` belongs to the caller, who decides when (and whether) to drop it.
	if target_existed {
		let _ = std::fs::remove_file(&backup);
	}
	Ok(())
}
/// Turn a rename failure into an actionable error, calling out the common
/// permission case (system install dirs need elevation).
pub(crate) fn rename_error(e: std::io::Error, target: &Path) -> ComposeError {
	if e.kind() == std::io::ErrorKind::PermissionDenied {
		ComposeError::Update(format!(
			"permission denied writing {}; re-run with elevated privileges \
			 (e.g. sudo) or set a writable install location",
			target.display()
		))
	} else {
		ComposeError::Update(format!(
			"failed to install update to {}: {e}",
			target.display()
		))
	}
}
