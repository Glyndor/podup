//! Pure configuration builders for container creation: restart policy, logging,
//! healthcheck, resource limits, and ulimits.
//!
//! Device, blkio, tmpfs, and label-file helpers live in `super::container::fields`.

use std::collections::HashMap;

use crate::compose::types::{
	Command as ComposeCommand, HealthCheck, LoggingConfig, RestartPolicy as ComposeRestart, Service,
};
use crate::error::ComposeError;
use crate::libpod::types::container::{HealthConfig, LogConfig};
use crate::size;

pub(crate) mod resources;
pub(super) use resources::{build_resource_limits, build_ulimits, cdi_devices};

// ---------------------------------------------------------------------------
// Restart policy
// ---------------------------------------------------------------------------

/// Returns `(policy_name, max_retry_tries)` for SpecGenerator.
pub(super) fn build_restart_policy(service: &Service) -> (Option<String>, Option<u64>) {
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
// Logging
// ---------------------------------------------------------------------------

/// Default log rotation applied when a compose file does not carry a
/// `logging:` block. `k8s-file` is the libpod default since Podman 4; pinning
/// it explicitly here stops the answer from drifting between podman versions
/// and distros. The 10 MB cap is enough for a week of typical service output
/// and small enough that a runaway loop will not exhaust the host. libpod
/// does not honour `max-file` on any path (see `man podman-run`), so it is
/// not part of the default; the user can override by writing their own
/// `logging:` in compose (#1417).
pub(crate) fn default_log_config() -> LogConfig {
	LogConfig {
		driver: Some("k8s-file".into()),
		size: Some(10 * 1024 * 1024),
		options: HashMap::new(),
	}
}

/// Resolve the `logging:` block into libpod's `LogConfig`. An absent compose
/// `logging:` maps to [`default_log_config`] so every container podup creates
/// has a rotation policy without the user having to set one. When the user
/// supplies a block, `max-size` is parsed into the typed [`LogConfig::size`]
/// field libpod actually reads rotation from (passing it inside `options` is
/// silently ignored, #1417); `max-file` is dropped with a warning because
/// libpod does not implement it. A malformed `max-size` is rejected with a
/// `PodmanError::Field` so the user gets a compose-flavoured error instead of
/// a 500 from libpod's JSON unmarshal.
pub(crate) fn build_log_config(
	service_name: &str,
	logging: Option<&LoggingConfig>,
) -> Result<Option<LogConfig>, ComposeError> {
	match logging {
		Some(l) => Ok(Some(translate_user_logging(service_name, l)?)),
		None => Ok(Some(default_log_config())),
	}
}

/// Translate a user-supplied `logging:` block into libpod's [`LogConfig`].
///
/// `max-size` is moved into the typed `size` field. `max-file` is dropped
/// with a warning: libpod does not implement it and would silently ignore
/// it if forwarded. The user-supplied value of `max-size` is parsed with the
/// same memory parser used elsewhere in the engine (`10m`, `1g`, plain
/// bytes); a malformed value is rejected with the service field name so the
/// error points at the compose key the user wrote (#1417).
fn translate_user_logging(
	service_name: &str,
	l: &LoggingConfig,
) -> Result<LogConfig, ComposeError> {
	let mut options = l.options.clone();
	let size = match options.remove("max-size") {
		Some(v) => match size::parse_memory(&v) {
			Some(bytes) => Some(bytes),
			None => {
				return Err(ComposeError::Podman(
					crate::libpod::validate::spec_field_error(
						service_name,
						"logging.options.max-size",
						&v,
						"must be a byte count (e.g. '10m', '1024', '1g'); \
						 libpod rejects an invalid `size` outright",
					),
				));
			}
		},
		None => None,
	};
	if options.remove("max-file").is_some() {
		tracing::warn!(
			"logging.options.max-file is ignored by libpod; \
			 remove it from your compose file or expect unbounded log growth"
		);
	}
	Ok(LogConfig {
		driver: l.driver.clone(),
		size,
		options,
	})
}

// ---------------------------------------------------------------------------
// Healthcheck
// ---------------------------------------------------------------------------

pub(super) fn build_healthcheck(hc: &HealthCheck) -> Option<HealthConfig> {
	if hc.is_disabled() {
		return Some(HealthConfig {
			test: Some(vec!["NONE".to_string()]),
			..Default::default()
		});
	}
	// Treat `None`, an empty exec list, and an empty shell string all as "no
	// test". libpod treats an empty Test as absent and inherits the image's
	// HEALTHCHECK; sending 30s/30s/3 there would overwrite the image's
	// values (#1893).
	let test = hc.test.as_ref().and_then(|cmd| match cmd {
		ComposeCommand::Shell(s) if !s.is_empty() => Some(vec!["CMD-SHELL".to_string(), s.clone()]),
		ComposeCommand::Exec(v) if !v.is_empty() => Some(v.clone()),
		_ => None,
	});
	let has_timings = hc.interval.is_some()
		|| hc.timeout.is_some()
		|| hc.retries.is_some()
		|| hc.start_period.is_some()
		|| hc.start_interval.is_some();
	// Podman 5.4.2 (still supported) inherits the image HEALTHCHECK only when
	// the whole `healthconfig` is absent from the request, so we must omit the
	// field entirely, not just leave every value `None`, when there is no
	// test and the user set no timing. Podman 5.7+ merges field-by-field and
	// would still inherit with a `None`-filled config, but emitting one anyway
	// would 30s/30s/3-overwrite older runtimes (#1893).
	if test.is_none() && !has_timings {
		return None;
	}
	// Apply the compose-spec defaults (interval 30s, timeout 30s, retries 3)
	// only when the user wrote a `test`. With a `test`, libpod would otherwise
	// leave the timings at 0s and the probe would fail with "exceeded timeout
	// of 0s". Without a `test` but with at least one user-set timing, we pass
	// the timing through and let libpod fill the rest from the image.
	const DEFAULT_NANOS: i64 = 30 * 1_000_000_000;
	let (interval, timeout, retries) = if test.is_some() {
		(
			Some(
				hc.interval
					.as_deref()
					.and_then(size::parse_duration_nanos)
					.unwrap_or(DEFAULT_NANOS),
			),
			Some(
				hc.timeout
					.as_deref()
					.and_then(size::parse_duration_nanos)
					.unwrap_or(DEFAULT_NANOS),
			),
			Some(hc.retries.map(|r| r as i64).unwrap_or(3)),
		)
	} else {
		(
			hc.interval.as_deref().and_then(size::parse_duration_nanos),
			hc.timeout.as_deref().and_then(size::parse_duration_nanos),
			hc.retries.map(|r| r as i64),
		)
	};
	Some(HealthConfig {
		test,
		interval,
		timeout,
		retries,
		start_period: hc
			.start_period
			.as_deref()
			.and_then(size::parse_duration_nanos),
		start_interval: hc
			.start_interval
			.as_deref()
			.and_then(size::parse_duration_nanos),
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
