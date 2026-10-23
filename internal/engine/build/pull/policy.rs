/// Map a compose `pull_policy:` value to the libpod images/pull `policy`
/// parameter. `if_not_present` is the spec alias for `missing`; `build` falls
/// back to `missing` here (its build behavior is handled by the caller). Returns
/// `None` for an unrecognized value so the caller can warn and default.
pub(in crate::engine) fn libpod_pull_policy(policy: Option<&str>) -> Option<&'static str> {
	match policy {
		Some("always") => Some("always"),
		Some("newer") => Some("newer"),
		Some("never") => Some("never"),
		None | Some("missing") | Some("if_not_present") | Some("build") => Some("missing"),
		Some(_) => None,
	}
}

/// Map a compose `pull_policy:` value to the libpod images/pull `policy`
/// parameter, rejecting an unrecognized value with a structured
/// [`PodmanError::Field`] that names both the compose service and the
/// offending value (#1443).
///
/// `service_name` is the compose service the policy was read from (e.g.
/// `"web"`); pass the empty string when the value came from the engine-wide
/// `--pull` override (no service context). The rejected value lands in the
/// error so a typo'd `pull_policy: alaways` on a specific service is reported
/// as `service.<name>: pull_policy: unknown pull policy "alaways" (value:
/// alaways)`: the actionable bit is which service and which value, not the
/// abstract policy name.
pub(crate) fn pull_policy_checked(
	policy: Option<&str>,
	service_name: &str,
) -> crate::error::Result<&'static str> {
	if let Some(p) = libpod_pull_policy(policy) {
		return Ok(p);
	}
	let value = policy.unwrap_or_default();
	let message = format!(
		"unknown pull policy {value:?}: accepted values are always, missing, newer, never \
		 (case-insensitive); if_not_present and build are accepted as aliases for missing"
	);
	Err(crate::error::ComposeError::Podman(
		crate::libpod::validate::spec_field_error(service_name, "pull_policy", value, message),
	))
}
