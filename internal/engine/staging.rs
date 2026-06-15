//! Per-user private base directory used by the project lock.
//!
//! [`staging_base`] returns a directory that must never be usable by another
//! local user. On unix it is created 0700 under `$XDG_RUNTIME_DIR` (fallback:
//! `temp_dir()/podup-<euid>`) and verified — real directory, owned by the
//! current user, no group/other bits — failing closed on anything else. On
//! Windows the base lives under the per-user temp directory, whose default
//! ACLs already restrict access to the owning user; only the non-symlink
//! directory check applies. [`reject_dangerous_secret_mode`] guards a compose
//! `mode:` before it is applied to a native secret.

// libc FFI (geteuid) is needed here; the block carries a soundness comment.
// Opt back into `unsafe` for this module only.
#![allow(unsafe_code)]

use crate::error::{ComposeError, Result};
use std::path::PathBuf;

#[cfg(unix)]
use std::path::Path;

/// Whether `name` is safe to use as a single path component and container
/// name prefix: non-empty, bounded, ASCII alphanumeric plus `-`/`_`/`.`,
/// not starting with a dot (rejects `.`, `..` and hidden directories).
pub fn is_safe_project_name(name: &str) -> bool {
	!name.is_empty()
		&& name.len() <= 128
		&& !name.starts_with('.')
		&& name
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Per-user staging base for inline secret/config content.
///
/// Prefers `$XDG_RUNTIME_DIR/podup` (per-user and 0700 by contract); falls
/// back to `temp_dir()/podup-<euid>`. The base sits in a world-writable
/// parent in the fallback case, so after creation it is verified to be a
/// real directory (not a symlink), owned by the current user, with no
/// group/other permission bits. Anything else aborts (fail closed) instead
/// of writing secret material under — or later deleting — a path another
/// local user may control.
#[cfg(unix)]
pub(super) fn staging_base() -> Result<PathBuf> {
	// SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
	let euid = unsafe { libc::geteuid() };

	let base = match std::env::var_os("XDG_RUNTIME_DIR") {
		Some(dir) if Path::new(&dir).is_absolute() => PathBuf::from(dir).join("podup"),
		_ => std::env::temp_dir().join(format!("podup-{euid}")),
	};

	ensure_private_dir(&base, euid)?;
	Ok(base)
}

/// Per-user staging base on Windows: `%TEMP%\podup`.
///
/// Unlike `/tmp` on unix, the Windows temp directory resolves under the
/// user profile and its default ACLs grant access to the owning user only,
/// so no ownership or permission-bit verification applies — just the
/// non-symlink directory check.
#[cfg(windows)]
pub(super) fn staging_base() -> Result<PathBuf> {
	let base = std::env::temp_dir().join("podup");
	std::fs::create_dir_all(&base).map_err(ComposeError::Io)?;
	let meta = std::fs::symlink_metadata(&base).map_err(ComposeError::Io)?;
	if !meta.is_dir() || meta.file_type().is_symlink() {
		return Err(ComposeError::Unsupported(format!(
			"staging directory {} is not a private directory owned by the \
             current user — refusing to use it",
			base.display()
		)));
	}
	Ok(base)
}

/// Reject permission bits that are dangerous on a secret/config file no matter
/// where it is materialised: any execute bit (`0o111`) and the setuid/setgid/
/// sticky bits (`0o7000`). A secret/config holds data, never code, so these are
/// a misconfiguration or an attack and are refused unconditionally. `ctx` names
/// the offending secret/config in the error message.
///
/// This does **not** reject group/world-read bits: a Podman-native secret is
/// materialised inside the container's own mount namespace and `0o444` is the
/// Podman/compose default, so a readable mode is legitimate for that path.
pub(super) fn reject_dangerous_secret_mode(mode: u32, ctx: &str) -> Result<()> {
	if mode & 0o111 != 0 {
		return Err(ComposeError::Unsupported(format!(
			"mode {mode:#o} for {ctx} sets an execute bit on a secret/config; \
			 a secret holds data, never code (use e.g. 0o400 or 0o444)"
		)));
	}
	if mode & (0o4000 | 0o2000 | 0o1000) != 0 {
		return Err(ComposeError::Unsupported(format!(
			"mode {mode:#o} for {ctx} sets setuid, setgid, or sticky bits on a \
			 secret/config; these are refused (use e.g. 0o400 or 0o444)"
		)));
	}
	Ok(())
}

/// Create `dir` (0700) if needed and require it to be a private directory.
///
/// `DirBuilder` does not reset permissions on a pre-existing directory, so
/// a leftover directory we own whose bits drifted is self-healed with a
/// chmod first — only if it is a real directory (never chmod through a
/// symlink; in the worst race the chmod tightens something we own to 0700).
/// `verify_private_dir` then rejects anything not ours (fail closed).
#[cfg(unix)]
fn ensure_private_dir(dir: &Path, euid: u32) -> Result<()> {
	use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

	std::fs::DirBuilder::new()
		.recursive(true)
		.mode(0o700)
		.create(dir)
		.map_err(ComposeError::Io)?;

	let meta = std::fs::symlink_metadata(dir).map_err(ComposeError::Io)?;
	if meta.is_dir() {
		std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
			.map_err(ComposeError::Io)?;
	}

	verify_private_dir(dir, euid)
}

/// Verify that `dir` is a non-symlink directory owned by `euid` with no
/// group/other permission bits.
#[cfg(unix)]
fn verify_private_dir(dir: &Path, euid: u32) -> Result<()> {
	use std::os::unix::fs::MetadataExt;

	let meta = std::fs::symlink_metadata(dir).map_err(ComposeError::Io)?;
	if !meta.is_dir() || meta.uid() != euid || meta.mode() & 0o077 != 0 {
		return Err(ComposeError::Unsupported(format!(
			"staging directory {} is not a private directory owned by the \
             current user — refusing to use it",
			dir.display()
		)));
	}
	Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod name_tests {
	use super::is_safe_project_name;

	#[test]
	fn safe_project_names_accepted() {
		for name in ["web", "my-app", "my_app", "app.v2", "A1"] {
			assert!(is_safe_project_name(name), "{name:?} must be accepted");
		}
	}

	#[test]
	fn unsafe_project_names_rejected() {
		let long = "a".repeat(129);
		for name in [
			"",
			".",
			"..",
			".hidden",
			"a/b",
			"../x",
			"a b",
			"a\0b",
			long.as_str(),
		] {
			assert!(!is_safe_project_name(name), "{name:?} must be rejected");
		}
	}
}

#[cfg(all(test, unix))]
mod staging_tests {
	use super::verify_private_dir;
	use std::os::unix::fs::PermissionsExt;

	#[test]
	fn private_dir_accepted() {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
			.expect("chmod");
		// SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
		let euid = unsafe { libc::geteuid() };
		assert!(verify_private_dir(dir.path(), euid).is_ok());
	}

	#[test]
	fn group_accessible_dir_rejected() {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o750))
			.expect("chmod");
		// SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
		let euid = unsafe { libc::geteuid() };
		assert!(verify_private_dir(dir.path(), euid).is_err());
	}

	#[test]
	fn foreign_owner_rejected() {
		let dir = tempfile::tempdir().expect("tempdir");
		std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
			.expect("chmod");
		// SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
		let other = unsafe { libc::geteuid() } + 1;
		assert!(verify_private_dir(dir.path(), other).is_err());
	}

	#[test]
	fn symlink_rejected() {
		let dir = tempfile::tempdir().expect("tempdir");
		let target = dir.path().join("real");
		let link = dir.path().join("link");
		std::fs::create_dir(&target).expect("mkdir");
		std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).expect("chmod");
		std::os::unix::fs::symlink(&target, &link).expect("symlink");
		// SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
		let euid = unsafe { libc::geteuid() };
		assert!(verify_private_dir(&link, euid).is_err());
	}

	#[test]
	fn regular_file_rejected() {
		let dir = tempfile::tempdir().expect("tempdir");
		let file = dir.path().join("file");
		std::fs::write(&file, b"x").expect("write");
		// SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
		let euid = unsafe { libc::geteuid() };
		assert!(verify_private_dir(&file, euid).is_err());
	}
}

#[cfg(all(test, unix))]
mod ensure_dir_tests {
	use super::ensure_private_dir;
	use std::os::unix::fs::{MetadataExt, PermissionsExt};

	#[test]
	fn creates_fresh_private_dir() {
		let root = tempfile::tempdir().expect("tempdir");
		let dir = root.path().join("base");
		// SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
		let euid = unsafe { libc::geteuid() };
		ensure_private_dir(&dir, euid).expect("fresh dir");
		let meta = std::fs::metadata(&dir).expect("metadata");
		assert_eq!(meta.mode() & 0o777, 0o700);
	}

	#[test]
	fn heals_drifted_permissions_on_owned_dir() {
		let root = tempfile::tempdir().expect("tempdir");
		let dir = root.path().join("base");
		std::fs::create_dir(&dir).expect("mkdir");
		std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
		// SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
		let euid = unsafe { libc::geteuid() };
		ensure_private_dir(&dir, euid).expect("healed dir");
		let meta = std::fs::metadata(&dir).expect("metadata");
		assert_eq!(meta.mode() & 0o777, 0o700);
	}

	#[test]
	fn symlinked_dir_is_rejected_not_healed() {
		let root = tempfile::tempdir().expect("tempdir");
		let target = root.path().join("real");
		let link = root.path().join("link");
		std::fs::create_dir(&target).expect("mkdir");
		std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).expect("chmod");
		std::os::unix::fs::symlink(&target, &link).expect("symlink");
		// SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
		let euid = unsafe { libc::geteuid() };
		assert!(ensure_private_dir(&link, euid).is_err());
		// Target permissions stay untouched — no chmod through the link.
		let meta = std::fs::metadata(&target).expect("metadata");
		assert_eq!(meta.mode() & 0o777, 0o755);
	}
}

#[cfg(test)]
mod reject_mode_tests {
	use super::reject_dangerous_secret_mode;

	#[test]
	fn data_modes_accepted() {
		// A secret holds data: read/write owner bits and the world-readable
		// default are all fine for a native secret.
		assert!(reject_dangerous_secret_mode(0o400, "s").is_ok());
		assert!(reject_dangerous_secret_mode(0o600, "s").is_ok());
		assert!(reject_dangerous_secret_mode(0o444, "s").is_ok());
	}

	#[test]
	fn execute_setuid_setgid_sticky_rejected() {
		for mode in [0o100, 0o500, 0o700, 0o4000, 0o2000, 0o1000] {
			assert!(
				reject_dangerous_secret_mode(mode, "s").is_err(),
				"{mode:#o} must be rejected"
			);
		}
	}
}

#[cfg(all(test, windows))]
mod windows_staging_tests {
	use super::staging_base;

	#[test]
	fn staging_base_is_a_directory() {
		let base = staging_base().expect("staging base");
		assert!(base.is_dir());
		assert!(base.ends_with("podup"));
	}
}
