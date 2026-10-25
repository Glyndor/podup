//! Resolve pull policies without mistaking image build time for pull time.
//!
//! Time policies use registry comparison because libpod has no last-pull clock.
//! `refresh` and `pull_refresh_after` are podup extensions.

use super::interval::parse_duration;

/// A resolved compose `pull_policy:` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PullPolicy {
	/// A fixed libpod policy, including the mapping of time policies to newer.
	Fixed(&'static str),
}

impl PullPolicy {
	/// The libpod pull policy this value maps onto.
	pub(crate) fn libpod(self) -> &'static str {
		let Self::Fixed(policy) = self;
		policy
	}
}

/// Whether a policy needs the last-pull tracking limitation warning.
pub(crate) fn is_time_based_pull_policy(policy: &str) -> bool {
	matches!(policy, "refresh" | "daily" | "weekly") || policy.starts_with("every_")
}

/// The warning shared by the runner and the Quadlet exporter.
pub(crate) fn pull_policy_warning(policy: &str) -> String {
	format!("pull_policy {policy:?} is treated as \"newer\": podup does not track when an image was last pulled")
}

/// Resolve a service policy and validate the extension interval even when unused.
/// Errors name the service, field and original value before any mutation.
pub(crate) fn pull_policy_checked(
	policy: Option<&str>,
	service_name: &str,
	refresh_after: Option<&str>,
) -> crate::error::Result<PullPolicy> {
	if let Some(value) = refresh_after {
		if parse_duration(value).is_none() {
			return Err(field_error(service_name, "pull_refresh_after", value));
		}
	}
	let value = policy.unwrap_or("missing");
	let mapped = match value {
		"always" => "always",
		"newer" => "newer",
		"never" => "never",
		"missing" | "if_not_present" | "build" => "missing",
		"daily" | "weekly" | "refresh" => "newer",
		_ if value
			.strip_prefix("every_")
			.is_some_and(|s| parse_duration(s).is_some()) =>
		{
			"newer"
		}
		_ => return Err(field_error(service_name, "pull_policy", value)),
	};
	Ok(PullPolicy::Fixed(mapped))
}

/// Keep accepted policy names and interval limits together in every refusal,
/// with a reason that matches what is actually wrong with the value.
fn field_error(service: &str, field: &str, value: &str) -> crate::error::ComposeError {
	let reason = if value.contains('$') {
		"the value still contains a placeholder, so a variable did not resolve"
	} else if field == "pull_policy" && value.starts_with("every_") {
		"the interval after every_ must be one or more number and unit pairs (w, d, h, m, s)"
	} else if field == "pull_refresh_after" {
		"the interval must be one or more number and unit pairs (w, d, h, m, s)"
	} else {
		"it is not a pull policy podup knows"
	};
	crate::error::ComposeError::Podman(crate::libpod::validate::spec_field_error(
		service,
		field,
		value,
		format!(
			"invalid {field} {value:?}: {reason}; accepted pull_policy values are always, missing, newer, \
			 never, if_not_present, build, daily, weekly, refresh and every_([0-9]+[wdhms])+; \
			 pull_refresh_after accepts ([0-9]+[wdhms])+; intervals must total at most {} seconds",
			u32::MAX
		),
	))
}

/// Shared preflight for all entry points, including direct library calls.
pub(crate) fn validate_pull_fields(
	file: &crate::compose::types::ComposeFile,
) -> crate::error::Result<()> {
	for (name, service) in &file.services {
		pull_policy_checked(
			service.pull_policy.as_deref(),
			name,
			service.pull_refresh_after.as_deref(),
		)?;
	}
	Ok(())
}
