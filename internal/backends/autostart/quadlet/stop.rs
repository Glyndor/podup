//! Distinguish a failed stop from a unit systemd left failed, and clean up both.
//!
//! Two outcomes end with the unit in `failed` even though nothing is left to
//! clean up: a unit that was already failed when the stop arrived, and a unit
//! whose container was killed with SIGKILL while it was being stopped (exit
//! 137), which systemd records as `Result=exit-code` even though the stop
//! itself worked and the container is gone. systemd reruns the cleanup hooks of
//! neither, so both run the same authorised cleanup and then clear the failed
//! state.

use super::drop_ins::DropIn;
use super::{checked, ComposeError, SystemCtl};

/// What `systemctl show` reported about a unit.
struct UnitState {
	/// `ActiveState`: the state systemd holds the unit in.
	active: String,
	/// `Result`: the outcome of the last run.
	result: String,
}

/// The `ActiveState` and the `Result` of a `show` response, when the response is
/// one that can be acted on.
///
/// Anything else is refused rather than guessed: a response that is not those
/// two properties, a state systemd does not have, or a `Result` that is not a
/// plain token.
fn parse_state(text: &str) -> Option<(&str, &str)> {
	if text.lines().count() != 2 {
		return None;
	}
	let active = text.lines().find_map(|s| s.strip_prefix("ActiveState="))?;
	if !matches!(
		active,
		"active"
			| "inactive"
			| "failed"
			| "activating"
			| "deactivating"
			| "reloading"
			| "maintenance"
			| "refreshing"
	) {
		return None;
	}
	let result = text.lines().find_map(|s| s.strip_prefix("Result="))?;
	if result.is_empty() || !result.bytes().all(|c| c.is_ascii_lowercase() || c == b'-') {
		return None;
	}
	Some((active, result))
}

/// Query the `ActiveState` and the `Result` of `service`.
fn unit_state<S: SystemCtl>(sc: &S, service: &str) -> crate::Result<UnitState> {
	let out = sc
		.systemctl(&["show", "--property=ActiveState,Result", service])
		.map_err(|e| {
			ComposeError::Autostart(format!(
				"cannot query ActiveState/Result for {service}: {e}"
			))
		})?;
	let text = std::str::from_utf8(&out.stdout).unwrap_or("");
	let parsed = if out.status.success() {
		parse_state(text)
	} else {
		None
	};
	let Some((active, result)) = parsed else {
		return Err(ComposeError::Autostart(format!(
			"cannot query ActiveState/Result for {service}: {:?}, status {}; {}",
			text,
			out.status,
			String::from_utf8_lossy(&out.stderr).trim()
		)));
	};
	Ok(UnitState {
		active: active.to_string(),
		result: result.to_string(),
	})
}

/// Run the authorised cleanup of `service` and clear the failed state it left.
///
/// systemd does not rerun the cleanup hooks of a unit that is already failed, so
/// the label-filtered command the drop-in carries is run here instead, and the
/// unit is reset once it succeeded.
fn cleanup<S: SystemCtl>(sc: &S, service: &str, protection: &[DropIn]) -> crate::Result<()> {
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
	checked(
		sc.systemctl(&["reset-failed", service]),
		&format!("reset-failed {service}"),
	)
}

/// Stop `service` and leave it in a state that lets its unit be retired.
pub(super) fn service<S: SystemCtl>(
	sc: &S,
	service: &str,
	protection: &[DropIn],
) -> crate::Result<()> {
	let before = unit_state(sc, service)?;
	checked(sc.systemctl(&["stop", service]), &format!("stop {service}"))?;
	if before.active == "failed" {
		// Stopping an already failed unit does not rerun its cleanup hooks.
		return cleanup(sc, service, protection);
	}
	let after = unit_state(sc, service)?;
	match (after.active.as_str(), after.result.as_str()) {
		("inactive", "success") => Ok(()),
		("failed", _) => {
			// The stop worked and the container is gone, but systemd left the
			// unit failed: the container was killed with SIGKILL (exit 137) on
			// its way down. Its cleanup hooks did not run for it, so they run
			// here.
			cleanup(sc, service, protection)
		}
		_ => Err(ComposeError::Autostart(format!(
			"stop {service}: ActiveState={:?}, Result={:?}; unit and protection retained",
			after.active, after.result
		))),
	}
}
