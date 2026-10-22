use std::path::{Path, PathBuf};

use crate::ComposeError;

/// Hard-link the current binary into a `.old` sibling so the swap and the
/// self-test can both find a recoverable copy of the previous binary.
///
/// On Windows the running `.exe` is `rename`d aside: a running executable
/// cannot be overwritten, and the swap moves the staged file over the
/// resulting empty path. On Unix the target stays in place: the staged
/// file is `rename`d over its inode atomically in `swap_into_place`, so
/// moving the target aside would only orphan the path between the link
/// and the swap. Keeping the target live is also what lets
/// `write_temp` read its permission bits before staging the new file
/// (`#1898`). The `.old` extension is shared across platforms so one
/// cleanup pass covers both.
pub(crate) fn move_target_aside(target: &Path) -> crate::Result<PathBuf> {
	let backup = target.with_extension("old");
	if backup == *target {
		return Err(ComposeError::Update(format!(
			"refusing to clobber the target with a sibling of the same path: {}",
			backup.display()
		)));
	}
	// Drop any stale leftover from a prior interrupted install. The
	// updater's next-run cleanup covers this too, but doing it here keeps
	// the rename atomic from the caller's perspective.
	if let Err(e) = std::fs::remove_file(&backup) {
		// ENOENT is fine; anything else is worth surfacing.
		if e.kind() != std::io::ErrorKind::NotFound {
			return Err(ComposeError::Update(format!(
				"cannot remove stale backup {}: {e}",
				backup.display()
			)));
		}
	}
	if target.exists() {
		#[cfg(windows)]
		{
			std::fs::rename(target, &backup).map_err(|e| {
				ComposeError::Update(format!(
					"cannot move the current binary aside before the swap ({} -> {}): {e}",
					target.display(),
					backup.display()
				))
			})?;
		}
		#[cfg(not(windows))]
		{
			// A hard link keeps the ORIGINAL inode at `.old` (mode, owner, xattrs,
			// ACLs, capabilities all intact), needs no read permission, and `link(2)`
			// never follows a symlink at the new path: it fails with EEXIST instead.
			// Nothing writes the target in place (the staged file is renamed over
			// it), so the two names only share an inode until that rename.
			if std::fs::hard_link(target, &backup).is_err() {
				// A filesystem without hard links: fall back to the rename this
				// function always did. The target is then absent until the swap.
				std::fs::rename(target, &backup).map_err(|e| {
					ComposeError::Update(format!(
						"cannot move the current binary aside before the swap ({} -> {}): {e}",
						target.display(),
						backup.display()
					))
				})?;
			}
		}
	}
	Ok(backup)
}

/// Restore the `.old` sibling back onto `target`. Best-effort: the rollback
/// path reports the original error to the user regardless, but a failure
/// here is logged at debug so the next-run cleanup can pick up where this
/// left off (the `.old` is still on disk and the self-test has already
/// failed).
pub(crate) fn restore_from_backup(target: &Path, backup: &Path) -> crate::Result<()> {
	if !backup.exists() {
		// Nothing to restore from (the install was a fresh deploy onto a
		// path that did not exist before). The swap that already ran
		// stands.
		return Ok(());
	}
	#[cfg(unix)]
	{
		use std::os::unix::fs::MetadataExt;
		// Renaming a hard link onto its own inode is a no-op that would leave `.old` behind.
		let same_file = match (
			std::fs::symlink_metadata(backup),
			std::fs::symlink_metadata(target),
		) {
			(Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
			_ => false,
		};
		if same_file {
			std::fs::remove_file(backup).map_err(|e| {
				ComposeError::Update(format!(
					"failed to remove the unused backup {}: {e}",
					backup.display()
				))
			})?;
			return Ok(());
		}
	}
	std::fs::rename(backup, target).map_err(|e| {
		ComposeError::Update(format!(
			"failed to roll back to the previous binary from {}: {e}",
			backup.display()
		))
	})
}
/// Best-effort removal of a `.old` backup [`swap_into_place`](super::swap_into_place) could not delete
/// immediately because the old process still held the file open. Call once at
/// the start of every updater run ([`crate::update::run`]): by then the
/// process that produced the backup has exited, so the file is no longer
/// locked and the leftover clears on this run rather than lingering until the
/// user happens to run another update. Silently does nothing if there is no
/// leftover, or if removal still fails for some other reason.
#[cfg(windows)]
pub(crate) fn cleanup_stale_backup() {
	if let Ok(exe) = std::env::current_exe() {
		let _ = std::fs::remove_file(exe.with_extension("old"));
	}
}
