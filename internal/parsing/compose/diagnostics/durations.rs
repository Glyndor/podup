//! Warnings for the duration fields podup reads but cannot translate.
//!
//! A duration that podup cannot read is a value the operator wrote and nobody
//! acted on: the container gets the default grace period, the default check
//! interval, the default window. Naming the key is enough to fix it, so the
//! value itself stays out of the message.

use super::ContainerUnits;

/// The duration fields of every service, with the compose key each one is
/// written under.
pub(super) fn untranslatable_durations(units: &ContainerUnits<'_>, out: &mut Vec<String>) {
	for (service, def) in units {
		if let Some(grace) = def.stop_grace_period.as_deref() {
			if crate::size::parse_duration_secs_rounded_up(grace).is_none() {
				out.push(untranslatable(service, "stop_grace_period"));
			}
		}
		if let Some(window) = def
			.deploy
			.as_ref()
			.and_then(|deploy| deploy.restart_policy.as_ref())
			.and_then(|policy| policy.window.as_deref())
		{
			if crate::size::parse_duration_secs_rounded_up(window).is_none() {
				out.push(untranslatable(service, "deploy.restart_policy.window"));
			}
		}
		if let Some(check) = def.healthcheck.as_ref() {
			for (key, value) in [
				("healthcheck.interval", check.interval.as_deref()),
				("healthcheck.timeout", check.timeout.as_deref()),
				("healthcheck.start_period", check.start_period.as_deref()),
				(
					"healthcheck.start_interval",
					check.start_interval.as_deref(),
				),
			] {
				if let Some(value) = value {
					if crate::size::parse_duration_nanos(value).is_none() {
						out.push(untranslatable(service, key));
					}
				}
			}
		}
	}
}

/// The warning for one duration podup cannot read.
fn untranslatable(service: &str, key: &str) -> String {
	format!(
		"service '{service}': {key} is not a duration podup can read (use a number of \
		 seconds or a value like 1m30s)"
	)
}
