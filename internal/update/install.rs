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
	asset_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Map an OS/ARCH pair to its release asset name. Split out from
/// [`platform_asset`] so the full matrix is testable without the host's values.
fn asset_for(os: &str, arch: &str) -> Option<&'static str> {
	match (os, arch) {
		("linux", "x86_64") => Some("podup-linux-x86_64"),
		("linux", "aarch64") => Some("podup-linux-arm64"),
		("macos", "aarch64") => Some("podup-darwin-arm64"),
		("macos", "x86_64") => Some("podup-darwin-x86_64"),
		("windows", "x86_64") => Some("podup-windows-x86_64.exe"),
		("windows", "aarch64") => Some("podup-windows-arm64.exe"),
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

/// Name of the system package manager that owns the running binary, if any.
///
/// Self-update replaces the executable in place, which would corrupt a package
/// manager's record of the installed file. When the running binary is tracked
/// by such a manager the caller refuses and redirects the user to it.
///
/// Only `dpkg`/`apt` is detected, on Linux. cargo-install layouts
/// (`~/.cargo/bin`, `/usr/local/bin`) are not owned by `dpkg` and update
/// normally. A missing `dpkg-query`, or a path no package owns, returns `None`.
#[cfg(target_os = "linux")]
pub fn managing_package_manager() -> Option<&'static str> {
	let exe = std::env::current_exe().ok()?;
	let path = std::fs::canonicalize(&exe).unwrap_or(exe);
	let output = std::process::Command::new("dpkg-query")
		.arg("-S")
		.arg(&path)
		.output()
		.ok()?;
	output.status.success().then_some("apt")
}

/// Non-Linux platforms have no supported package-manager-managed install yet.
#[cfg(not(target_os = "linux"))]
pub fn managing_package_manager() -> Option<&'static str> {
	None
}

/// Error returned when the running binary is managed by package manager `pm`.
pub fn package_managed_error(pm: &str) -> ComposeError {
	ComposeError::Update(format!(
		"this podup was installed by {pm}; update it with your package manager \
		 (e.g. `apt upgrade podup`) rather than `podup update`, which would break \
		 the package's record of the file"
	))
}

/// Replace the currently running executable with `new_bytes`. Returns the path
/// that was updated. The caller MUST have verified `new_bytes` first.
pub fn install_binary(new_bytes: &[u8]) -> crate::Result<PathBuf> {
	let exe = std::env::current_exe()
		.map_err(|e| ComposeError::Update(format!("cannot locate current executable: {e}")))?;
	// Resolve symlinks so we replace the real file, not a symlink pointing at it.
	let target = std::fs::canonicalize(&exe).unwrap_or(exe);
	// Keep the current binary in memory so a failed self-test can roll back. The
	// signature already proves the new bytes are authentic; the self-test guards
	// the install mechanics (a partial write, an arch/ABI mismatch the asset name
	// didn't catch) by confirming the replacement actually runs.
	let backup = std::fs::read(&target).ok();
	install_at(&target, new_bytes)?;
	if let Err(e) = self_test(&target) {
		return match backup {
			Some(old) => {
				install_at(&target, &old)?;
				Err(ComposeError::Update(format!(
					"the updated binary failed its self-test ({e}); rolled back to the \
					 previous version"
				)))
			}
			None => Err(ComposeError::Update(format!(
				"the updated binary failed its self-test ({e}) and no backup was \
				 available to roll back"
			))),
		};
	}
	Ok(target)
}

/// Confirm a freshly-installed binary runs by invoking `--version`, bounded by a
/// timeout so a hung binary can't wedge the updater. Output is discarded; only
/// the exit status matters.
fn self_test(target: &Path) -> crate::Result<()> {
	use std::process::{Command, Stdio};
	use std::time::{Duration, Instant};

	let mut child = Command::new(target)
		.arg("--version")
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.map_err(|e| ComposeError::Update(format!("could not run the updated binary: {e}")))?;

	let deadline = Instant::now() + Duration::from_secs(10);
	loop {
		match child.try_wait() {
			Ok(Some(status)) if status.success() => return Ok(()),
			Ok(Some(status)) => {
				return Err(ComposeError::Update(format!(
					"updated binary exited with {status} on --version"
				)))
			}
			Ok(None) => {
				if Instant::now() >= deadline {
					let _ = child.kill();
					return Err(ComposeError::Update(
						"updated binary did not respond to --version within 10s".to_string(),
					));
				}
				std::thread::sleep(Duration::from_millis(50));
			}
			Err(e) => {
				return Err(ComposeError::Update(format!(
					"waiting on the updated binary failed: {e}"
				)))
			}
		}
	}
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
	#[cfg(not(unix))]
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
	fn platform_asset_covers_every_release_target() {
		// Pins the OS/ARCH → asset mapping to the full `release.yml` build
		// matrix so a newly added prebuilt (or a dropped arm) is caught here
		// instead of failing self-update silently in the field.
		let expected = [
			(("linux", "x86_64"), "podup-linux-x86_64"),
			(("linux", "aarch64"), "podup-linux-arm64"),
			(("macos", "aarch64"), "podup-darwin-arm64"),
			(("macos", "x86_64"), "podup-darwin-x86_64"),
			(("windows", "x86_64"), "podup-windows-x86_64.exe"),
			(("windows", "aarch64"), "podup-windows-arm64.exe"),
		];
		for ((os, arch), asset) in expected {
			assert_eq!(
				asset_for(os, arch),
				Some(asset),
				"self-update mapping drifted for {os}/{arch}"
			);
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

	#[cfg(unix)]
	#[test]
	fn install_at_strips_setuid_from_target_mode() {
		use std::os::unix::fs::PermissionsExt;
		let dir = tempfile::tempdir().unwrap();
		let target = dir.path().join("podup");
		std::fs::write(&target, b"old").unwrap();
		// A tampered/setuid target must not propagate its special bits onto the
		// freshly installed binary.
		std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o4755)).unwrap();

		install_at(&target, b"new").unwrap();
		let mode = std::fs::metadata(&target).unwrap().permissions().mode();
		assert_eq!(mode & 0o7000, 0, "setuid/setgid/sticky must be stripped");
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
	fn install_at_fails_when_target_dir_is_missing() {
		// A target whose parent directory does not exist must fail (the sibling
		// temp cannot be created) and must not leave anything behind.
		let dir = tempfile::tempdir().unwrap();
		let missing = dir.path().join("no-such-subdir");
		let target = missing.join("podup");
		assert!(install_at(&target, b"data").is_err());
		assert!(!missing.exists(), "must not create the missing parent dir");
	}

	#[cfg(unix)]
	#[test]
	fn self_test_passes_for_a_zero_exit_and_fails_otherwise() {
		use std::os::unix::fs::PermissionsExt;
		let dir = tempfile::tempdir().unwrap();
		let mk = |name: &str, body: &str| {
			let p = dir.path().join(name);
			std::fs::write(&p, body).unwrap();
			std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
			p
		};
		// A binary that exits 0 on --version passes; one that exits non-zero fails.
		let ok = mk("ok", "#!/bin/sh\nexit 0\n");
		let bad = mk("bad", "#!/bin/sh\nexit 1\n");
		assert!(self_test(&ok).is_ok());
		assert!(self_test(&bad).is_err());
		// A non-executable / missing target is a spawn error, not a panic.
		assert!(self_test(&dir.path().join("nope")).is_err());
	}

	#[test]
	fn require_platform_asset_is_consistent() {
		match (platform_asset(), require_platform_asset()) {
			(Some(a), Ok(b)) => assert_eq!(a, b),
			(None, Err(_)) => {}
			_ => panic!("platform_asset and require_platform_asset disagree"),
		}
	}

	#[test]
	fn package_managed_error_names_the_manager() {
		let e = package_managed_error("apt");
		match e {
			ComposeError::Update(msg) => {
				assert!(msg.contains("apt"));
				assert!(msg.contains("podup update"));
			}
			_ => panic!("expected an Update error"),
		}
	}

	#[test]
	fn test_binary_is_not_package_managed() {
		// The test runner binary lives under target/, which no package owns, so
		// detection must not false-positive and block updates for normal builds.
		assert_eq!(managing_package_manager(), None);
	}

	#[test]
	fn rename_error_calls_out_permission_and_generic_cases() {
		let target = Path::new("/usr/local/bin/podup");
		// A permission error nudges the user toward elevation.
		let perm = rename_error(
			std::io::Error::from(std::io::ErrorKind::PermissionDenied),
			target,
		);
		match perm {
			ComposeError::Update(msg) => {
				assert!(msg.contains("permission denied"));
				assert!(msg.contains("sudo"));
			}
			_ => panic!("expected an Update error"),
		}
		// Any other error reports the underlying failure verbatim.
		let other = rename_error(std::io::Error::other("disk full"), target);
		match other {
			ComposeError::Update(msg) => {
				assert!(msg.contains("failed to install update"));
				assert!(msg.contains("disk full"));
			}
			_ => panic!("expected an Update error"),
		}
	}
}
