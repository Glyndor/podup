//! Pure configuration builders for container creation: restart policy, logging,
//! healthcheck, resource limits, and ulimits.
//!
//! Device, blkio, tmpfs, and label-file helpers live in `super::container::fields`.

use crate::compose::types::{
	Command as ComposeCommand, HealthCheck, LoggingConfig, RestartPolicy as ComposeRestart, Service,
};
use crate::libpod::types::container::{HealthConfig, LogConfig};
use crate::size;

mod resources;
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
		// under any circumstance, which docker-compose maps to `always` — not
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
		return (Some(name.to_string()), drp.max_attempts.map(|n| n as u64));
	}
	(None, None)
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

pub(super) fn build_log_config(logging: Option<&LoggingConfig>) -> Option<LogConfig> {
	logging.map(|l| LogConfig {
		driver: l.driver.clone(),
		options: l.options.clone(),
	})
}

// ---------------------------------------------------------------------------
// Healthcheck
// ---------------------------------------------------------------------------

pub(super) fn build_healthcheck(hc: &HealthCheck) -> HealthConfig {
	if hc.is_disabled() {
		return HealthConfig {
			test: Some(vec!["NONE".to_string()]),
			..Default::default()
		};
	}
	let test = hc.test.as_ref().map(|cmd| match cmd {
		ComposeCommand::Shell(s) => vec!["CMD-SHELL".to_string(), s.clone()],
		ComposeCommand::Exec(v) => v.clone(),
	});
	// Apply the compose-spec defaults for any field the user omitted. Podman's
	// API does NOT default these: a missing `Timeout` is taken as 0s, which makes
	// every probe fail with "exceeded timeout of 0s" so the container is stuck
	// `starting`; a missing/zero `Interval` disables the periodic check. Match
	// docker-compose — interval 30s, timeout 30s, retries 3 (start_period 0).
	const DEFAULT_NANOS: i64 = 30 * 1_000_000_000;
	HealthConfig {
		test,
		interval: Some(
			hc.interval
				.as_deref()
				.and_then(size::parse_duration_nanos)
				.unwrap_or(DEFAULT_NANOS),
		),
		timeout: Some(
			hc.timeout
				.as_deref()
				.and_then(size::parse_duration_nanos)
				.unwrap_or(DEFAULT_NANOS),
		),
		retries: Some(hc.retries.map(|r| r as i64).unwrap_or(3)),
		start_period: hc
			.start_period
			.as_deref()
			.and_then(size::parse_duration_nanos),
		start_interval: hc
			.start_interval
			.as_deref()
			.and_then(size::parse_duration_nanos),
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::compose::types::{
		Command as ComposeCommand, HealthCheck, LoggingConfig, RestartPolicy as ComposeRestart,
		Service,
	};

	fn default_service() -> Service {
		Service::default()
	}

	// --- restart policy ---

	#[test]
	fn restart_policy_no() {
		let mut svc = default_service();
		svc.restart = Some(ComposeRestart::No);
		let (name, tries) = build_restart_policy(&svc);
		assert_eq!(name.as_deref(), Some("no"));
		assert!(tries.is_none());
	}

	#[test]
	fn restart_policy_always() {
		let mut svc = default_service();
		svc.restart = Some(ComposeRestart::Always);
		let (name, _) = build_restart_policy(&svc);
		assert_eq!(name.as_deref(), Some("always"));
	}

	#[test]
	fn restart_policy_on_failure_with_retries() {
		let mut svc = default_service();
		svc.restart = Some(ComposeRestart::OnFailure {
			max_attempts: Some(3),
		});
		let (name, tries) = build_restart_policy(&svc);
		assert_eq!(name.as_deref(), Some("on-failure"));
		assert_eq!(tries, Some(3));
	}

	#[test]
	fn restart_policy_unless_stopped() {
		let mut svc = default_service();
		svc.restart = Some(ComposeRestart::UnlessStopped);
		let (name, _) = build_restart_policy(&svc);
		assert_eq!(name.as_deref(), Some("unless-stopped"));
	}

	#[test]
	fn restart_policy_none_when_absent() {
		let (name, _) = build_restart_policy(&default_service());
		assert!(name.is_none());
	}

	#[test]
	fn restart_policy_from_deploy_on_failure() {
		use crate::compose::types::{DeployConfig, DeployRestartPolicy};
		let mut svc = default_service();
		svc.deploy = Some(DeployConfig {
			restart_policy: Some(DeployRestartPolicy {
				condition: Some("on-failure".into()),
				max_attempts: Some(5),
				..Default::default()
			}),
			..Default::default()
		});
		let (name, tries) = build_restart_policy(&svc);
		assert_eq!(name.as_deref(), Some("on-failure"));
		assert_eq!(tries, Some(5));
	}

	#[test]
	fn restart_policy_from_deploy_none_condition() {
		use crate::compose::types::{DeployConfig, DeployRestartPolicy};
		let mut svc = default_service();
		svc.deploy = Some(DeployConfig {
			restart_policy: Some(DeployRestartPolicy {
				condition: Some("none".into()),
				..Default::default()
			}),
			..Default::default()
		});
		let (name, _) = build_restart_policy(&svc);
		assert_eq!(name.as_deref(), Some("no"));
	}

	#[test]
	fn restart_policy_from_deploy_any_maps_to_always() {
		use crate::compose::types::{DeployConfig, DeployRestartPolicy};
		let mut svc = default_service();
		svc.deploy = Some(DeployConfig {
			restart_policy: Some(DeployRestartPolicy {
				condition: Some("any".into()),
				..Default::default()
			}),
			..Default::default()
		});
		let (name, _) = build_restart_policy(&svc);
		assert_eq!(name.as_deref(), Some("always"));
	}

	#[test]
	fn restart_policy_from_deploy_default_condition_is_always() {
		// An unset `condition` defaults to `any` per the compose spec → `always`.
		use crate::compose::types::{DeployConfig, DeployRestartPolicy};
		let mut svc = default_service();
		svc.deploy = Some(DeployConfig {
			restart_policy: Some(DeployRestartPolicy::default()),
			..Default::default()
		});
		let (name, _) = build_restart_policy(&svc);
		assert_eq!(name.as_deref(), Some("always"));
	}

	// --- log config ---

	#[test]
	fn log_config_none_when_absent() {
		assert!(build_log_config(None).is_none());
	}

	#[test]
	fn log_config_driver_only() {
		let logging = LoggingConfig {
			driver: Some("json-file".into()),
			options: Default::default(),
		};
		let cfg = build_log_config(Some(&logging)).unwrap();
		assert_eq!(cfg.driver.as_deref(), Some("json-file"));
		assert!(cfg.options.is_empty());
	}

	#[test]
	fn log_config_with_options() {
		let mut opts = std::collections::HashMap::new();
		opts.insert("max-size".into(), "10m".into());
		let logging = LoggingConfig {
			driver: Some("json-file".into()),
			options: opts,
		};
		let cfg = build_log_config(Some(&logging)).unwrap();
		assert_eq!(cfg.options["max-size"], "10m");
	}

	// --- healthcheck ---

	#[test]
	fn healthcheck_disabled() {
		let hc = HealthCheck {
			disable: Some(true),
			..Default::default()
		};
		let cfg = build_healthcheck(&hc);
		assert_eq!(cfg.test.unwrap(), vec!["NONE"]);
	}

	#[test]
	fn healthcheck_shell_command() {
		let hc = HealthCheck {
			test: Some(ComposeCommand::Shell(
				"curl -f http://localhost/health".into(),
			)),
			interval: Some("30s".into()),
			timeout: Some("10s".into()),
			retries: Some(3),
			..Default::default()
		};
		let cfg = build_healthcheck(&hc);
		let test = cfg.test.unwrap();
		assert_eq!(test[0], "CMD-SHELL");
		assert!(test[1].contains("curl"));
		assert_eq!(cfg.retries, Some(3));
	}

	#[test]
	fn healthcheck_exec_command() {
		let hc = HealthCheck {
			test: Some(ComposeCommand::Exec(vec![
				"curl".into(),
				"-f".into(),
				"http://localhost".into(),
			])),
			..Default::default()
		};
		let cfg = build_healthcheck(&hc);
		let test = cfg.test.unwrap();
		assert_eq!(test[0], "curl");
	}

	#[test]
	fn healthcheck_applies_compose_defaults_when_omitted() {
		// A healthcheck with only a `test` must still get interval/timeout/retries:
		// Podman treats a missing Timeout as 0s, which makes every probe fail with
		// "exceeded timeout of 0s" and the container is stuck `starting`.
		let hc = HealthCheck {
			test: Some(ComposeCommand::Exec(vec!["true".into()])),
			..Default::default()
		};
		let cfg = build_healthcheck(&hc);
		assert_eq!(cfg.interval, Some(30 * 1_000_000_000));
		assert_eq!(cfg.timeout, Some(30 * 1_000_000_000));
		assert_eq!(cfg.retries, Some(3));
	}

	#[test]
	fn healthcheck_honors_explicit_interval_and_timeout() {
		let hc = HealthCheck {
			test: Some(ComposeCommand::Exec(vec!["true".into()])),
			interval: Some("2s".into()),
			timeout: Some("5s".into()),
			retries: Some(7),
			..Default::default()
		};
		let cfg = build_healthcheck(&hc);
		assert_eq!(cfg.interval, Some(2 * 1_000_000_000));
		assert_eq!(cfg.timeout, Some(5 * 1_000_000_000));
		assert_eq!(cfg.retries, Some(7));
	}
}
