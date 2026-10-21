//! The pre-action advisories: linger (so the user manager, and the stack,
//! survives logout and starts at boot) and `XDG_RUNTIME_DIR` (so `systemctl
//! --user`'s session bus is reachable). Both are warn-only, never fail.

use super::systemctl::SystemCtl;

/// The current login user, for `loginctl` and linger queries. Read from the
/// environment to avoid an `unsafe` `getuid` FFI call.
pub(super) fn current_user() -> Option<String> {
	std::env::var("USER")
		.ok()
		.or_else(|| std::env::var("LOGNAME").ok())
		.filter(|s| !s.is_empty())
}

/// Whether linger is enabled for `user` (so the user manager, and the stack,
/// survives logout and starts at boot). Parses `loginctl show-user <user>
/// --value --property=Linger`, treating any error/unexpected output as "off".
pub(super) fn linger_enabled<S: SystemCtl>(sc: &S, user: &str) -> bool {
	match sc.loginctl(&["show-user", user, "--value", "--property=Linger"]) {
		Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
			.trim()
			.eq_ignore_ascii_case("yes"),
		_ => false,
	}
}

/// Advisory when linger is off: without it the user manager is not started at
/// boot, so the unit will not bring the stack up until first login. Returns the
/// message to print, or `None` when linger is already enabled.
pub(super) fn linger_warning<S: SystemCtl>(sc: &S) -> Option<String> {
	let user = current_user()?;
	if linger_enabled(sc, &user) {
		return None;
	}
	Some(format!(
		"linger is not enabled for {user}; the stack will not start at boot until you run:\n    \
		 loginctl enable-linger {user}"
	))
}

/// Advisory when `XDG_RUNTIME_DIR` is unset: `systemctl --user` needs a live user
/// session bus, which is otherwise missing (so the calls below will likely fail).
/// Returns the message to print, or `None` when the variable is set.
pub(super) fn runtime_dir_warning() -> Option<String> {
	let present = std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|s| !s.is_empty());
	if present {
		return None;
	}
	Some(
		"XDG_RUNTIME_DIR is not set; `systemctl --user` needs an active user session. \
		 Open one (e.g. `machinectl shell <user>@`) or export XDG_RUNTIME_DIR before retrying."
			.to_string(),
	)
}

/// Print the linger and runtime-dir advisories to stderr (warn, never fail).
pub(super) fn emit_guards<S: SystemCtl>(sc: &S) {
	for warning in [linger_warning(sc), runtime_dir_warning()]
		.into_iter()
		.flatten()
	{
		tracing::warn!("{warning}");
	}
}
