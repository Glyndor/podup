//! Where `podup autostart` writes its unit files, and how the slowest container's
//! `stop_grace_period` is rolled up into the `TimeoutStopSec=` value.
//!
//! The user-scope unit directory is derived from `XDG_CONFIG_HOME` so the install
//! path stays inside the user's home tree; nothing here is system-managed and no
//! step requires root. The grace-period roll-up is the value the unit renderer
//! reads to bound its `ExecStop` above systemd's 90s default.

use std::path::{Path, PathBuf};

use crate::compose::types::ComposeFile;

/// `${XDG_CONFIG_HOME:-~/.config}`. Falls back to `$HOME/.config`, then `.config`
/// in the working directory if even `HOME` is unset (so a path is always formed).
pub(in super::super) fn config_home() -> PathBuf {
	if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()) {
		return PathBuf::from(x);
	}
	match std::env::var_os("HOME").filter(|s| !s.is_empty()) {
		Some(home) => PathBuf::from(home).join(".config"),
		None => PathBuf::from(".config"),
	}
}

/// Directory that holds `systemctl --user` unit files.
pub(in super::super) fn unit_dir() -> PathBuf {
	config_home().join("systemd").join("user")
}

/// The unit's file name: `podup-<project>.service`. The project name is validated
/// as a safe path component before reaching here, so it cannot escape `unit_dir`.
pub(in super::super) fn unit_file_name(project: &str) -> String {
	format!("podup-{project}.service")
}

/// Full path to the unit file for a project.
pub(in super::super) fn unit_path(project: &str) -> PathBuf {
	unit_dir().join(unit_file_name(project))
}

/// The longest `stop_grace_period` across a compose file's services, in seconds.
///
/// systemd bounds `ExecStop` independently of what runs inside it, and its
/// default `DefaultTimeoutStopUSec` is 90s. `podup stop` honours each
/// container's own grace period, so a stack whose slowest container needs more
/// than that stops cleanly when a human runs it and gets killed mid-stop at
/// reboot; the difference only shows up during an unattended shutdown, which
/// is the worst version of it.
///
/// `None` when no service sets one, or when none parses, so the unit simply
/// omits `TimeoutStopSec=` and keeps systemd's default. An unparseable duration
/// is skipped rather than defaulted: the value is validated elsewhere, and
/// guessing a timeout from a malformed one would be worse than not setting it.
pub fn max_stop_grace_secs(file: &ComposeFile) -> Option<u64> {
	file.services
		.values()
		.filter_map(|s| s.stop_grace_period.as_deref())
		.filter_map(crate::size::parse_duration_secs)
		.max()
}

/// Read the unit file's permission bits, on Unix. Other platforms have no POSIX
/// mode, so this is always `None` there.
#[cfg(unix)]
pub(in super::super) fn file_mode(path: &Path) -> Option<u32> {
	use std::os::unix::fs::PermissionsExt;
	std::fs::metadata(path).ok().map(|m| m.permissions().mode())
}

#[cfg(not(unix))]
pub(in super::super) fn file_mode(_path: &Path) -> Option<u32> {
	None
}
