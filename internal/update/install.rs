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
			 https://github.com/Glyndor-net/podup/releases",
			std::env::consts::OS,
			std::env::consts::ARCH
		))
	})
}

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
	// Keep the current binary in memory so a failed self-test can roll back. The
	// signature already proves the new bytes are authentic; the self-test guards
	// the install mechanics (a partial write, an arch/ABI mismatch the asset name
	// didn't catch) and pins the reported version against the resolved tag.
	let backup = std::fs::read(&target).ok();
	install_at(&target, new_bytes)?;
	if let Err(e) = self_test(&target, expected_version) {
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

/// Confirm a freshly-installed binary runs and reports `expected_version` by
/// invoking `--version`, bounded by a timeout so a hung binary can't wedge the
/// updater. The version check closes the rollback window: a replayed older
/// (signed) release fails here and is rolled back.
fn self_test(target: &Path, expected_version: &str) -> crate::Result<()> {
	use std::io::Read;
	use std::process::{Command, Stdio};
	use std::time::{Duration, Instant};

	let mut child = Command::new(target)
		.arg("--version")
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.map_err(|e| ComposeError::Update(format!("could not run the updated binary: {e}")))?;

	let deadline = Instant::now() + Duration::from_secs(10);
	let status = loop {
		match child.try_wait() {
			Ok(Some(status)) => break status,
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
	};
	if !status.success() {
		return Err(ComposeError::Update(format!(
			"updated binary exited with {status} on --version"
		)));
	}
	// `--version` output is a single short line; reading after exit is safe
	// (it fits the pipe buffer, so the child never blocks on a full pipe).
	let mut out = String::new();
	if let Some(mut stdout) = child.stdout.take() {
		let _ = stdout.read_to_string(&mut out);
	}
	let reported_matches = out
		.split_whitespace()
		.any(|t| t == expected_version || t.trim_start_matches('v') == expected_version);
	if !reported_matches {
		return Err(ComposeError::Update(format!(
			"updated binary reports {:?} instead of the resolved release version \
			 {expected_version} — possible release-metadata tampering (rollback)",
			out.trim()
		)));
	}
	Ok(())
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

	/// Write an executable stub script and return its path.
	#[cfg(unix)]
	fn write_stub(dir: &Path, name: &str, body: &str) -> PathBuf {
		use std::os::unix::fs::PermissionsExt;
		let p = dir.join(name);
		std::fs::write(&p, body).unwrap();
		std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
		p
	}

	/// Run [`self_test`], retrying while the spawn hits ETXTBSY: a concurrent
	/// test thread forking between our write and exec briefly holds the
	/// script's write fd open, which makes exec fail spuriously.
	#[cfg(unix)]
	fn self_test_retrying(target: &Path, expected: &str) -> crate::Result<()> {
		for _ in 0..100 {
			match self_test(target, expected) {
				Err(e) if format!("{e}").contains("Text file busy") => {
					std::thread::sleep(std::time::Duration::from_millis(10));
				}
				other => return other,
			}
		}
		self_test(target, expected)
	}

	#[cfg(unix)]
	#[test]
	fn self_test_passes_for_a_zero_exit_and_fails_otherwise() {
		let dir = tempfile::tempdir().unwrap();
		// A binary that exits 0 and reports the expected version passes; a
		// non-zero exit fails.
		let ok = write_stub(
			dir.path(),
			"ok",
			"#!/bin/sh\necho \"podup 9.9.9\"\nexit 0\n",
		);
		let bad = write_stub(dir.path(), "bad", "#!/bin/sh\nexit 1\n");
		assert!(self_test_retrying(&ok, "9.9.9").is_ok());
		assert!(self_test_retrying(&bad, "9.9.9").is_err());
		// A non-executable / missing target is a spawn error, not a panic.
		assert!(self_test(&dir.path().join("nope"), "9.9.9").is_err());
	}

	#[cfg(unix)]
	#[test]
	fn self_test_rejects_a_version_mismatch() {
		let dir = tempfile::tempdir().unwrap();
		// A genuinely-signed but *older* replayed release exits 0 yet reports the
		// wrong version — the rollback gate must reject it.
		let p = write_stub(
			dir.path(),
			"older",
			"#!/bin/sh\necho \"podup 1.0.0\"\nexit 0\n",
		);
		let err = self_test_retrying(&p, "9.9.9").unwrap_err();
		let msg = format!("{err}");
		assert!(msg.contains("rollback"), "{msg}");
		// A `v`-prefixed report still matches its unprefixed expectation.
		let v = write_stub(
			dir.path(),
			"vprefixed",
			"#!/bin/sh\necho \"podup v9.9.9\"\nexit 0\n",
		);
		assert!(self_test_retrying(&v, "9.9.9").is_ok());
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
