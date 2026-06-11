//! Atomic, in-place replacement of the running binary.
//!
//! The verified new bytes are written to a temporary file in the *same*
//! directory as the target (so the final swap is a same-filesystem rename, which
//! is atomic) and then moved into place. On Unix the running binary's inode can
//! be replaced directly. On Windows a running `.exe` cannot be overwritten, so
//! the current file is renamed aside (`.old`) first and cleaned up on the next
//! run.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::ComposeError;

/// The release asset name for the platform this binary was built for. Mirrors
/// the `release.yml` build matrix exactly.
pub fn platform_asset() -> Option<&'static str> {
	match (std::env::consts::OS, std::env::consts::ARCH) {
		("linux", "x86_64") => Some("podup-linux-x86_64"),
		("linux", "aarch64") => Some("podup-linux-arm64"),
		("macos", "aarch64") => Some("podup-darwin-arm64"),
		("macos", "x86_64") => Some("podup-darwin-x86_64"),
		("windows", "x86_64") => Some("podup-windows-x86_64.exe"),
		_ => None,
	}
}

/// Resolve the asset for the current platform or fail with a clear message.
pub fn require_platform_asset() -> crate::Result<&'static str> {
	platform_asset().ok_or_else(|| {
		ComposeError::Update(format!(
			"self-update is not supported on {}/{}; reinstall manually from \
			 https://github.com/Glyndor/podup/releases",
			std::env::consts::OS,
			std::env::consts::ARCH
		))
	})
}

/// Replace the currently running executable with `new_bytes`. Returns the path
/// that was updated. The caller MUST have verified `new_bytes` first.
pub fn install_binary(new_bytes: &[u8]) -> crate::Result<PathBuf> {
	let exe = std::env::current_exe()
		.map_err(|e| ComposeError::Update(format!("cannot locate current executable: {e}")))?;
	// Resolve symlinks so we replace the real file, not a symlink pointing at it.
	let target = std::fs::canonicalize(&exe).unwrap_or(exe);
	install_at(&target, new_bytes)?;
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

/// Write the new bytes to `tmp`, copy `target`'s permission bits (default 0755
/// on Unix when the target does not yet exist), and flush to disk.
fn write_temp(tmp: &Path, new_bytes: &[u8], target: &Path) -> crate::Result<()> {
	let mut f = std::fs::File::create(tmp).map_err(|e| {
		ComposeError::Update(format!("cannot write update to {}: {e}", tmp.display()))
	})?;
	f.write_all(new_bytes).map_err(ComposeError::Io)?;
	f.flush().map_err(ComposeError::Io)?;

	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		let mode = std::fs::metadata(target)
			.map(|m| m.permissions().mode())
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
fn swap_into_place(tmp: &Path, target: &Path) -> crate::Result<()> {
	std::fs::rename(tmp, target).map_err(|e| rename_error(e, target))
}

#[cfg(windows)]
fn swap_into_place(tmp: &Path, target: &Path) -> crate::Result<()> {
	// A running .exe cannot be overwritten, but it can be renamed. Move it aside,
	// put the new binary in place, then best-effort delete the old one (it may
	// still be locked while running — the next run cleans it up).
	let backup = target.with_extension("old");
	let _ = std::fs::remove_file(&backup);
	if target.exists() {
		std::fs::rename(target, &backup).map_err(|e| rename_error(e, target))?;
	}
	if let Err(e) = std::fs::rename(tmp, target) {
		// Roll back so the user is not left without a binary.
		let _ = std::fs::rename(&backup, target);
		return Err(rename_error(e, target));
	}
	let _ = std::fs::remove_file(&backup);
	Ok(())
}

/// Turn a rename failure into an actionable error, calling out the common
/// permission case (system install dirs need elevation).
fn rename_error(e: std::io::Error, target: &Path) -> ComposeError {
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn platform_asset_matches_known_targets() {
		// Whatever host runs the tests, the asset (if any) must be one of the
		// release matrix names.
		if let Some(asset) = platform_asset() {
			assert!(asset.starts_with("podup-"));
		}
	}

	#[test]
	fn install_at_replaces_contents() {
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("podup");
		std::fs::write(&target, b"old version").unwrap();

		install_at(&target, b"new version").unwrap();
		assert_eq!(std::fs::read(&target).unwrap(), b"new version");
	}

	#[test]
	fn install_at_creates_when_absent() {
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("podup");
		install_at(&target, b"fresh").unwrap();
		assert_eq!(std::fs::read(&target).unwrap(), b"fresh");
	}

	#[cfg(unix)]
	#[test]
	fn install_at_preserves_executable_mode() {
		use std::os::unix::fs::PermissionsExt;
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("podup");
		std::fs::write(&target, b"old").unwrap();
		std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();

		install_at(&target, b"new").unwrap();
		let mode = std::fs::metadata(&target).unwrap().permissions().mode();
		assert_eq!(mode & 0o777, 0o755);
	}

	#[test]
	fn install_at_leaves_no_temp_files() {
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("podup");
		install_at(&target, b"data").unwrap();
		let leftovers: Vec<_> = std::fs::read_dir(dir.path())
			.unwrap()
			.filter_map(|e| e.ok())
			.filter(|e| e.file_name().to_string_lossy().contains("update-"))
			.collect();
		assert!(leftovers.is_empty(), "temp file left behind");
	}

	#[test]
	fn require_platform_asset_is_consistent() {
		match (platform_asset(), require_platform_asset()) {
			(Some(a), Ok(b)) => assert_eq!(a, b),
			(None, Err(_)) => {}
			_ => panic!("platform_asset and require_platform_asset disagree"),
		}
	}
}
