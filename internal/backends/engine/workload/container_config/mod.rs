//! Pure configuration builders for container creation: restart policy, logging,
//! healthcheck, resource limits, and ulimits.
//!
//! Device, blkio, tmpfs, and label-file helpers live in `crate::engine::container::fields`.

use crate::compose::types::{
	Command as ComposeCommand, HealthCheck, RestartPolicy as ComposeRestart, Service,
};
use crate::libpod::types::container::HealthConfig;
use crate::libpod::types::image::ImageHealthcheck;
use crate::size;

mod logging;
pub(crate) mod resources;
pub(crate) use logging::build_log_config;
pub(in crate::backends::engine) use resources::{
	build_resource_limits, build_ulimits, cdi_devices,
};

// ---------------------------------------------------------------------------
// Restart policy
// ---------------------------------------------------------------------------

/// Returns `(policy_name, max_retry_tries)` for SpecGenerator.
pub(in crate::backends::engine) fn build_restart_policy(
	service: &Service,
) -> (Option<String>, Option<u64>) {
	if let Some(r) = &service.restart {
		let (name, tries) = match r {
			ComposeRestart::No => ("no", None),
			ComposeRestart::Always => ("always", None),
			ComposeRestart::OnFailure { max_attempts } => {
				("on-failure", max_attempts.map(|n| n as u64))
			}
			ComposeRestart::UnlessStopped => ("unless-stopped", None),
		};
		return (Some(name.to_string()), tries);
	}
	if let Some(drp) = service
		.deploy
		.as_ref()
		.and_then(|d| d.restart_policy.as_ref())
	{
		// Compose `restart_policy.condition`: `any` (the default) means restart
		// under any circumstance, which docker-compose maps to `always`, not
		// `unless-stopped` (the latter would skip restarts after an explicit
		// stop, diverging from docker-compose).
		let name = match drp.condition.as_deref().unwrap_or("any") {
			"none" => "no",
			"on-failure" => "on-failure",
			"any" => "always",
			other => {
				tracing::warn!(
					"deploy.restart_policy.condition '{other}' is not recognized \
					 (expected none/on-failure/any); falling back to 'unless-stopped'"
				);
				"unless-stopped"
			}
		};
		// Podman only honours a retry cap (`RestartRetries`) when the policy is
		// `on-failure`. Under any other policy the cap is silently dropped by the
		// backend, which would turn a bounded "restart at most N times" spec into an
		// unbounded restart. Only forward `max_attempts` for `on-failure`, and warn
		// if the user set one under a condition where it cannot take effect.
		let tries = if name == "on-failure" {
			drp.max_attempts.map(|n| n as u64)
		} else {
			if drp.max_attempts.is_some() {
				tracing::warn!(
					"deploy.restart_policy.max_attempts is ignored unless condition \
					 is 'on-failure' (current condition resolves to '{name}')"
				);
			}
			None
		};
		return (Some(name.to_string()), tries);
	}
	(None, None)
}

// ---------------------------------------------------------------------------
// Healthcheck
// ---------------------------------------------------------------------------

pub(in crate::backends::engine) fn build_healthcheck(hc: &HealthCheck) -> Option<HealthConfig> {
	build_healthcheck_inheriting(hc, None)
}

/// The healthcheck of a service, with every value its compose file leaves out
/// taken from the healthcheck its image declares.
///
/// The Compose Specification says a service's healthcheck "has the same default
/// values as the HEALTHCHECK Dockerfile instruction set by the service's Docker
/// image" and that the compose file can override them. Declaring only a `test`
/// therefore overrides the test and nothing else: the interval, timeout, start
/// period and retries stay the image's. libpod does not merge them for a request
/// that carries a `test`, so they are filled in here from `image`, the
/// `Healthcheck` of the image inspect. Only what neither the compose file nor the
/// image gives falls back to the specification's 30s / 30s / 3.
///
/// A compose file that gives timings and no `test` sends no test: libpod merges
/// field by field in that case and keeps the image's probe (measured on Podman
/// 6.2.0, 2026-11-22).
pub(in crate::backends::engine) fn build_healthcheck_inheriting(
	hc: &HealthCheck,
	image: Option<&ImageHealthcheck>,
) -> Option<HealthConfig> {
	if hc.is_disabled() {
		return Some(HealthConfig {
			test: Some(vec!["NONE".to_string()]),
			..Default::default()
		});
	}
	// Only `None` and an empty exec list count as "no test": libpod treats an
	// empty Test as absent and inherits the image probe. An empty shell string
	// stays an explicit `["CMD-SHELL", ""]` probe, as it always was, because
	// Podman runs it (#1893).
	let test = hc.test.as_ref().and_then(|cmd| match cmd {
		ComposeCommand::Shell(s) => Some(vec!["CMD-SHELL".to_string(), s.clone()]),
		ComposeCommand::Exec(v) if !v.is_empty() => Some(v.clone()),
		_ => None,
	});
	let has_timings = hc.interval.is_some()
		|| hc.timeout.is_some()
		|| hc.retries.is_some()
		|| hc.start_period.is_some()
		|| hc.start_interval.is_some();
	// With no test and no timing there is nothing to say: the field is left out
	// of the request so the image HEALTHCHECK applies untouched (#1893).
	if test.is_none() && !has_timings {
		return None;
	}
	// Without a test of its own the request only merges into an image that has a
	// probe; an image that has none (or turns it off) lends nothing.
	let lender = image.filter(|i| {
		test.is_some()
			|| i.test
				.as_ref()
				.and_then(|t| t.first())
				.is_some_and(|mode| mode != "NONE")
	});
	let from_image =
		|pick: fn(&ImageHealthcheck) -> Option<i64>| lender.and_then(pick).filter(|v| *v > 0);
	let nanos = |value: &Option<String>| value.as_deref().and_then(size::parse_duration_nanos);
	let mut interval = nanos(&hc.interval).or_else(|| from_image(|i| i.interval));
	let mut timeout = nanos(&hc.timeout).or_else(|| from_image(|i| i.timeout));
	let mut retries = hc
		.retries
		.map(|r| r as i64)
		.or_else(|| from_image(|i| i.retries));
	// The specification's defaults, only for what the file and the image both
	// leave out, and only with a test: libpod would otherwise leave the timings
	// at 0s and the probe would fail with "exceeded timeout of 0s".
	if test.is_some() {
		const DEFAULT_NANOS: i64 = 30 * 1_000_000_000;
		interval = interval.or(Some(DEFAULT_NANOS));
		timeout = timeout.or(Some(DEFAULT_NANOS));
		retries = retries.or(Some(3));
	}
	Some(HealthConfig {
		test,
		interval,
		timeout,
		retries,
		start_period: nanos(&hc.start_period).or_else(|| from_image(|i| i.start_period)),
		start_interval: nanos(&hc.start_interval).or_else(|| from_image(|i| i.start_interval)),
	})
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "log_config_tests.rs"]
mod log_config_tests;

#[cfg(test)]
#[path = "log_options_report_tests.rs"]
mod log_options_report_tests;

#[cfg(test)]
#[path = "log_tag_tests.rs"]
mod log_tag_tests;

#[cfg(test)]
#[path = "healthcheck_inherit_tests.rs"]
mod healthcheck_inherit_tests;
