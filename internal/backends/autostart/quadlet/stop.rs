//! Distinguish a failed stop from a historical failure whose hooks systemd will not retry.

use super::drop_ins::DropIn;
use super::{checked, ComposeError, SystemCtl};

pub(super) fn service<S: SystemCtl>(
	sc: &S,
	service: &str,
	protection: &[DropIn],
) -> crate::Result<()> {
	let out = sc
		.systemctl(&["show", "--property=ActiveState,Result", service])
		.map_err(|e| {
			ComposeError::Autostart(format!(
				"cannot query ActiveState/Result for {service}: {e}"
			))
		})?;
	let text = std::str::from_utf8(&out.stdout).unwrap_or("");
	let active = text.lines().find_map(|s| s.strip_prefix("ActiveState="));
	let result = text.lines().find_map(|s| s.strip_prefix("Result="));
	if !out.status.success()
		|| text.lines().count() != 2
		|| !matches!(
			active,
			Some(
				"active"
					| "inactive"
					| "failed"
					| "activating"
					| "deactivating"
					| "reloading"
					| "maintenance"
					| "refreshing"
			)
		)
		|| !result.is_some_and(|s| {
			!s.is_empty() && s.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
		}) {
		return Err(ComposeError::Autostart(format!(
			"cannot query ActiveState/Result for {service}: {:?}, status {}; {}",
			text,
			out.status,
			String::from_utf8_lossy(&out.stderr).trim()
		)));
	}
	checked(sc.systemctl(&["stop", service]), &format!("stop {service}"))?;
	if active == Some("failed") {
		// Stopping an already failed unit does not rerun its cleanup hooks.
		let command = protection
			.iter()
			.find(|drop_in| drop_in.service == service)
			.and_then(DropIn::cleanup_command)
			.ok_or_else(|| {
				ComposeError::Autostart(format!(
					"cleanup {service}: no authorised project-label cleanup; unit and protection retained"
				))
			})?;
		let out = sc.cleanup(&command).map_err(|e| {
			ComposeError::Autostart(format!(
				"cleanup {service}: {e}; unit and protection retained; pending command: {command}"
			))
		})?;
		if !out.status.success() {
			return Err(ComposeError::Autostart(format!(
				"cleanup {service}: status {}; {}; unit and protection retained; pending command: {command}",
				out.status, String::from_utf8_lossy(&out.stderr).trim()
			)));
		}
		return checked(
			sc.systemctl(&["reset-failed", service]),
			&format!("reset-failed {service}"),
		);
	}
	let out = sc
		.systemctl(&["show", "--property=Result", "--value", service])
		.map_err(|e| ComposeError::Autostart(format!("cannot query Result for {service}: {e}")))?;
	let result = String::from_utf8_lossy(&out.stdout);
	if !out.status.success() || result.trim() != "success" {
		return Err(ComposeError::Autostart(format!(
			"stop {service}: Result={:?}, query status {}; {}",
			result.trim(),
			out.status,
			String::from_utf8_lossy(&out.stderr).trim()
		)));
	}
	Ok(())
}
