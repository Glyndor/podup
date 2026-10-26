//! The `SystemCtl` seam: every external `systemctl --user` / `loginctl` call
//! goes through this trait so the install/uninstall/status logic is unit-
//! testable without a live systemd. The real implementation shells out; tests
//! substitute a fake that records the argument vectors and returns canned
//! output. `checked` is the small wrapper that turns a `systemctl` invocation
//! into a `Result`, mapping a launch failure or a non-zero exit into a clear
//! autostart error naming the action.

use std::io;
use std::process::{Command, Output};

use crate::ComposeError;

/// Seam over the external `systemctl --user` / `loginctl` commands. The real impl
/// shells out; tests substitute a fake that records the argument vectors and
/// returns canned output, so install/uninstall/status are exercised without
/// touching the host's systemd.
pub trait SystemCtl {
	/// Run `systemctl --user <args>`.
	fn systemctl(&self, args: &[&str]) -> io::Result<Output>;
	/// Run `loginctl <args>`.
	fn loginctl(&self, args: &[&str]) -> io::Result<Output>;
}

/// The production [`SystemCtl`]: invokes the real `systemctl --user` and
/// `loginctl` binaries.
pub struct RealSystemCtl;

impl SystemCtl for RealSystemCtl {
	fn systemctl(&self, args: &[&str]) -> io::Result<Output> {
		Command::new("systemctl").arg("--user").args(args).output()
	}

	fn loginctl(&self, args: &[&str]) -> io::Result<Output> {
		Command::new("loginctl").args(args).output()
	}
}

/// Turn a `systemctl` invocation result into a `Result`, mapping a launch failure
/// or a non-zero exit into a clear autostart error naming the action.
pub(in super::super) fn checked(res: io::Result<Output>, what: &str) -> crate::Result<()> {
	let out = res.map_err(|e| {
		ComposeError::Autostart(format!("failed to run `systemctl --user {what}`: {e}"))
	})?;
	if out.status.success() {
		return Ok(());
	}
	let stderr = String::from_utf8_lossy(&out.stderr);
	Err(ComposeError::Autostart(format!(
		"`systemctl --user {what}` failed: {}",
		stderr.trim()
	)))
}
