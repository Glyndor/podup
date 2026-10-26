//! Atomic, in-place replacement of the running binary.
//!
//! The verified new bytes are written to a temporary file in the *same*
//! directory as the target (so the final swap is a same-filesystem rename, which
//! is atomic) and then moved into place. On Unix the running binary's inode can
//! be replaced directly. On Windows a running `.exe` cannot be overwritten, so
//! the current file is renamed aside (`.old`) first. The immediate best-effort
//! delete of that backup can fail while the old process still holds the file
//! open; when it does, the leftover is removed at the start of the next
//! updater run (`cleanup_stale_backup`, called from [`crate::update::run`]),
//! not merely "the next run" of the binary in general.

use std::path::{Path, PathBuf};

use crate::ComposeError;

mod backup;
mod platform;
mod self_test;
mod swap;

#[cfg(windows)]
pub(crate) use backup::cleanup_stale_backup;
pub(crate) use backup::{move_target_aside, restore_from_backup};
pub(crate) use platform::asset_for;
pub use platform::{platform_asset, require_platform_asset};
#[cfg(unix)]
pub(crate) use self_test::is_text_file_busy;
pub(crate) use self_test::{self_test, spawn_version_probe};
pub(crate) use swap::{rename_error, swap_into_place, write_temp};

/// Replace the currently running executable with `new_bytes`. Returns the path
/// that was updated. The caller MUST have verified `new_bytes` first.
///
/// `expected_version` is the resolved release version (no `v` prefix). The
/// self-test confirms the installed binary actually reports it: the signed
/// manifest binds asset bytes but not the release tag, so without this check a
/// man-in-the-middle able to spoof the release metadata could replay an older,
/// genuinely-signed release as the "latest" one (a rollback attack).
pub fn install_binary(new_bytes: &[u8], expected_version: &str) -> crate::Result<PathBuf> {
	let exe = std::env::current_exe()
		.map_err(|e| ComposeError::Update(format!("cannot locate current executable: {e}")))?;
	// Resolve symlinks so we replace the real file, not a symlink pointing at it.
	// Fail closed: replacing the symlink itself would orphan the real target.
	let target = std::fs::canonicalize(&exe).map_err(|e| {
		ComposeError::Update(format!(
			"cannot resolve the real path of {}: {e}",
			exe.display()
		))
	})?;
	// Hard-link the current binary into `.old` BEFORE the swap so a failed
	// self-test always has a rollback target on disk. On Unix the target
	// stays in place (its mode is read by `write_temp`); on Windows the
	// running `.exe` is renamed aside (`#1898`).
	let backup = move_target_aside(&target)?;
	if let Err(e) = install_at(&target, new_bytes) {
		// The swap itself failed: restore the old binary before reporting.
		let _ = restore_from_backup(&target, &backup);
		return Err(e);
	}
	if let Err(e) = self_test(&target, expected_version) {
		// Roll back to the original binary. The backup is on disk, so a
		// kill inside `restore_from_backup` would still leave the user
		// with a recoverable `.old` to fall back on.
		restore_from_backup(&target, &backup)?;
		return Err(ComposeError::Update(format!(
			"the updated binary failed its self-test ({e}); rolled back to the \
			 previous version"
		)));
	}
	// Best-effort remove the `.old`; a kill here just leaves a recoverable
	// sibling that the next run will tidy up.
	let _ = std::fs::remove_file(&backup);
	Ok(target)
}

/// Write `new_bytes` to a sibling temp file and atomically move it onto
/// `target`, preserving the target's permissions. Factored out of
/// [`install_binary`] so the swap is testable against an arbitrary path.
pub fn install_at(target: &Path, new_bytes: &[u8]) -> crate::Result<()> {
	let dir = target.parent().ok_or_else(|| {
		ComposeError::Update(format!(
			"target {} has no parent directory",
			target.display()
		))
	})?;

	let file_name = target
		.file_name()
		.map(|n| n.to_string_lossy().into_owned())
		.unwrap_or_else(|| "podup".to_string());
	let tmp = dir.join(format!(".{file_name}.update-{}", std::process::id()));

	write_temp(&tmp, new_bytes, target).inspect_err(|_| {
		let _ = std::fs::remove_file(&tmp);
	})?;

	if let Err(e) = swap_into_place(&tmp, target) {
		let _ = std::fs::remove_file(&tmp);
		return Err(e);
	}
	Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod platform_tests;

#[cfg(test)]
mod backup_tests;

#[cfg(test)]
mod self_test_tests;

#[cfg(test)]
mod swap_tests;

#[cfg(all(test, unix))]
mod swap_hostile_tests;

#[cfg(all(test, unix))]
mod hostile_tests;

#[cfg(all(test, unix))]
mod self_test_pipe_tests;
