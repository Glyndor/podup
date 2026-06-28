//! Detect whether the running binary is owned by a system package manager.
//!
//! Self-update replaces the executable in place, which would corrupt a package
//! manager's record of the installed file. When the running binary is tracked by
//! such a manager the caller refuses and redirects the user to it.

#[cfg(target_os = "linux")]
use std::path::Path;

use crate::ComposeError;

/// Name of the system package manager that owns the running binary, if any.
///
/// Only `dpkg`/`apt` is detected, on Linux. cargo-install layouts
/// (`~/.cargo/bin`, `/usr/local/bin`) are not owned by `dpkg` and update
/// normally. A path no package owns returns `None`.
#[cfg(target_os = "linux")]
pub fn managing_package_manager() -> Option<&'static str> {
	let exe = std::env::current_exe().ok()?;
	let path = std::fs::canonicalize(&exe).unwrap_or(exe);
	dpkg_owns(&path).then_some("apt")
}

/// Whether dpkg's database records `path` as belonging to an installed package.
///
/// The primary check is `dpkg-query -S <path>`. Crucially this must not *fail
/// open*: if the helper cannot be spawned (missing, not executable, or denied)
/// we do not assume the file is unmanaged — that would let self-update clobber an
/// apt-owned binary and desync the dpkg database. Instead we fall back to reading
/// dpkg's on-disk file lists directly. A host with no dpkg database at all
/// (`/var/lib/dpkg/info` absent — e.g. Fedora or a cargo-install) is genuinely
/// not Debian-managed, so both paths report `false` and update proceeds.
#[cfg(target_os = "linux")]
fn dpkg_owns(path: &Path) -> bool {
	match std::process::Command::new("dpkg-query")
		.arg("-S")
		.arg(path)
		.output()
	{
		// dpkg-query ran to completion: trust its verdict (success == owned).
		Ok(output) => output.status.success(),
		// dpkg-query could not be spawned. Don't fail open — consult dpkg's
		// own file lists, which exist only on a Debian-family system.
		Err(_) => dpkg_lists_contain(path),
	}
}

/// Fallback ownership check: scan dpkg's installed-file manifests
/// (`/var/lib/dpkg/info/*.list`) for an exact line matching `path`.
#[cfg(target_os = "linux")]
fn dpkg_lists_contain(path: &Path) -> bool {
	dpkg_lists_contain_in(Path::new("/var/lib/dpkg/info"), path)
}

/// Core of [`dpkg_lists_contain`], parameterised on the info directory so it can
/// be tested against a fixture without a real dpkg database. Returns `false`
/// when the directory is absent (not a Debian host).
#[cfg(target_os = "linux")]
fn dpkg_lists_contain_in(info_dir: &Path, path: &Path) -> bool {
	let Ok(entries) = std::fs::read_dir(info_dir) else {
		return false; // no dpkg database → not Debian-managed
	};
	let needle = path.to_string_lossy();
	for entry in entries.flatten() {
		let p = entry.path();
		if p.extension().and_then(|e| e.to_str()) != Some("list") {
			continue;
		}
		if let Ok(contents) = std::fs::read_to_string(&p) {
			if contents.lines().any(|line| line == needle) {
				return true;
			}
		}
	}
	false
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

#[cfg(test)]
mod tests {
	use super::*;

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

	#[cfg(target_os = "linux")]
	#[test]
	fn dpkg_lists_fallback_matches_an_owned_path() {
		// Simulate dpkg's info dir: a `.list` file naming the binary's path means
		// the file is package-managed and self-update must refuse.
		let dir = tempfile::tempdir().unwrap();
		let owned = Path::new("/usr/bin/podup");
		std::fs::write(
			dir.path().join("podup.list"),
			"/.\n/usr/bin\n/usr/bin/podup\n",
		)
		.unwrap();
		// A non-`.list` file with the same name must be ignored.
		std::fs::write(dir.path().join("other.md5sums"), "/usr/bin/podup\n").unwrap();

		assert!(dpkg_lists_contain_in(dir.path(), owned));
		assert!(!dpkg_lists_contain_in(
			dir.path(),
			Path::new("/usr/bin/somethingelse")
		));
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn dpkg_lists_fallback_is_false_without_a_database() {
		// No dpkg info directory (e.g. Fedora, cargo-install): genuinely not
		// Debian-managed, so the fallback reports unowned and update proceeds.
		let dir = tempfile::tempdir().unwrap();
		let absent = dir.path().join("no-such-info-dir");
		assert!(!dpkg_lists_contain_in(&absent, Path::new("/usr/bin/podup")));
	}
}
