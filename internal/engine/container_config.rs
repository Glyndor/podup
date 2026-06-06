//! Pure configuration builders for container creation: restart policy, logging,
//! healthcheck, resource limits, and ulimits.
//!
//! Device, blkio, tmpfs, and label-file helpers live in [`super::container_misc`].

use bollard::models::{
	HealthConfig, HostConfigLogConfig, ResourcesUlimits, RestartPolicy as BollardRestart,
	RestartPolicyNameEnum,
};

use crate::compose::types::{
	Command as ComposeCommand, HealthCheck, LoggingConfig, RestartPolicy as ComposeRestart, Service,
};
use crate::size;

// ---------------------------------------------------------------------------
// Restart policy
// ---------------------------------------------------------------------------

pub(super) fn build_restart_policy(service: &Service) -> Option<BollardRestart> {
	if let Some(r) = &service.restart {
		return Some(match r {
			ComposeRestart::No => BollardRestart {
				name: Some(RestartPolicyNameEnum::NO),
				maximum_retry_count: None,
			},
			ComposeRestart::Always => BollardRestart {
				name: Some(RestartPolicyNameEnum::ALWAYS),
				maximum_retry_count: None,
			},
			ComposeRestart::OnFailure { max_attempts } => BollardRestart {
				name: Some(RestartPolicyNameEnum::ON_FAILURE),
				maximum_retry_count: max_attempts.map(|n| n as i64),
			},
			ComposeRestart::UnlessStopped => BollardRestart {
				name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
				maximum_retry_count: None,
			},
		});
	}
	if let Some(drp) = service
		.deploy
		.as_ref()
		.and_then(|d| d.restart_policy.as_ref())
	{
		let name = match drp.condition.as_deref().unwrap_or("any") {
			"none" => RestartPolicyNameEnum::NO,
			"on-failure" => RestartPolicyNameEnum::ON_FAILURE,
			_ => RestartPolicyNameEnum::UNLESS_STOPPED,
		};
		return Some(BollardRestart {
			name: Some(name),
			maximum_retry_count: drp.max_attempts.map(|n| n as i64),
		});
	}
	None
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

pub(super) fn build_log_config(logging: Option<&LoggingConfig>) -> Option<HostConfigLogConfig> {
	logging.map(|l| HostConfigLogConfig {
		typ: l.driver.clone(),
		config: if l.options.is_empty() {
			None
		} else {
			Some(l.options.clone())
		},
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
	HealthConfig {
		test,
		interval: hc.interval.as_deref().and_then(size::parse_duration_nanos),
		timeout: hc.timeout.as_deref().and_then(size::parse_duration_nanos),
		retries: hc.retries.map(|r| r as i64),
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
// Resource limits
// ---------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
pub(super) fn resolve_resources(
	service: &Service,
) -> (
	Option<i64>,
	Option<i64>,
	Option<i64>,
	Option<i64>,
	Option<i64>,
	Option<i64>,
	Option<i64>,
) {
	let mut memory = service.mem_limit.as_deref().and_then(size::parse_memory);
	let mut mem_reservation = service
		.mem_reservation
		.as_deref()
		.and_then(size::parse_memory);
	let memswap = service
		.memswap_limit
		.as_deref()
		.and_then(size::parse_memory);
	let mut nano_cpus = service.cpus.as_deref().and_then(size::parse_cpus);
	let cpu_quota = service.cpu_quota;
	let cpu_period = service.cpu_period.map(|p| p as i64);
	let mut pids_limit = service.pids_limit;

	if let Some(deploy) = &service.deploy {
		if let Some(res) = &deploy.resources {
			if let Some(limits) = &res.limits {
				if memory.is_none() {
					memory = limits.memory.as_deref().and_then(size::parse_memory);
				}
				if nano_cpus.is_none() {
					nano_cpus = limits.cpus.as_deref().and_then(size::parse_cpus);
				}
				if pids_limit.is_none() {
					pids_limit = limits.pids.map(|p| p as i64);
				}
			}
			if let Some(reserv) = &res.reservations {
				if mem_reservation.is_none() {
					mem_reservation = reserv.memory.as_deref().and_then(size::parse_memory);
				}
			}
		}
	}

	(
		memory,
		mem_reservation,
		memswap,
		nano_cpus,
		cpu_quota,
		cpu_period,
		pids_limit,
	)
}

pub(super) fn build_ulimits(service: &Service) -> Vec<ResourcesUlimits> {
	service
		.ulimits
		.iter()
		.map(|(name, cfg)| ResourcesUlimits {
			name: Some(name.clone()),
			soft: Some(cfg.soft()),
			hard: Some(cfg.hard()),
		})
		.collect()
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
		let p = build_restart_policy(&svc).unwrap();
		assert_eq!(p.name, Some(RestartPolicyNameEnum::NO));
		assert!(p.maximum_retry_count.is_none());
	}

	#[test]
	fn restart_policy_always() {
		let mut svc = default_service();
		svc.restart = Some(ComposeRestart::Always);
		let p = build_restart_policy(&svc).unwrap();
		assert_eq!(p.name, Some(RestartPolicyNameEnum::ALWAYS));
	}

	#[test]
	fn restart_policy_on_failure_with_retries() {
		let mut svc = default_service();
		svc.restart = Some(ComposeRestart::OnFailure { max_attempts: Some(3) });
		let p = build_restart_policy(&svc).unwrap();
		assert_eq!(p.name, Some(RestartPolicyNameEnum::ON_FAILURE));
		assert_eq!(p.maximum_retry_count, Some(3));
	}

	#[test]
	fn restart_policy_unless_stopped() {
		let mut svc = default_service();
		svc.restart = Some(ComposeRestart::UnlessStopped);
		let p = build_restart_policy(&svc).unwrap();
		assert_eq!(p.name, Some(RestartPolicyNameEnum::UNLESS_STOPPED));
	}

	#[test]
	fn restart_policy_none_when_absent() {
		assert!(build_restart_policy(&default_service()).is_none());
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
		let p = build_restart_policy(&svc).unwrap();
		assert_eq!(p.name, Some(RestartPolicyNameEnum::ON_FAILURE));
		assert_eq!(p.maximum_retry_count, Some(5));
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
		let p = build_restart_policy(&svc).unwrap();
		assert_eq!(p.name, Some(RestartPolicyNameEnum::NO));
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
		assert_eq!(cfg.typ.as_deref(), Some("json-file"));
		assert!(cfg.config.is_none());
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
		assert_eq!(cfg.config.unwrap()["max-size"], "10m");
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
			test: Some(ComposeCommand::Shell("curl -f http://localhost/health".into())),
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

	// --- resource resolution ---

	#[test]
	fn resolve_resources_empty_service() {
		let (mem, mem_res, memswap, nano_cpus, cpu_quota, cpu_period, pids) =
			resolve_resources(&default_service());
		assert!(mem.is_none());
		assert!(mem_res.is_none());
		assert!(memswap.is_none());
		assert!(nano_cpus.is_none());
		assert!(cpu_quota.is_none());
		assert!(cpu_period.is_none());
		assert!(pids.is_none());
	}

	#[test]
	fn resolve_resources_mem_limit() {
		let mut svc = default_service();
		svc.mem_limit = Some("512m".into());
		let (mem, _, _, _, _, _, _) = resolve_resources(&svc);
		assert_eq!(mem, Some(512 * 1024 * 1024));
	}

	#[test]
	fn resolve_resources_deploy_overrides() {
		use crate::compose::types::{DeployConfig, ResourceSpec, ResourcesConfig};
		let mut svc = default_service();
		svc.deploy = Some(DeployConfig {
			resources: Some(ResourcesConfig {
				limits: Some(ResourceSpec {
					memory: Some("256m".into()),
					..Default::default()
				}),
				reservations: None,
			}),
			..Default::default()
		});
		let (mem, _, _, _, _, _, _) = resolve_resources(&svc);
		assert_eq!(mem, Some(256 * 1024 * 1024));
	}

	// --- ulimits ---

	#[test]
	fn build_ulimits_single_value() {
		use crate::compose::types::UlimitConfig;
		let mut svc = default_service();
		svc.ulimits.insert("nofile".to_string(), UlimitConfig::Single(1024));
		let ul = build_ulimits(&svc);
		assert_eq!(ul.len(), 1);
		assert_eq!(ul[0].name.as_deref(), Some("nofile"));
		assert_eq!(ul[0].soft, Some(1024));
		assert_eq!(ul[0].hard, Some(1024));
	}

	#[test]
	fn build_ulimits_pair() {
		use crate::compose::types::UlimitConfig;
		let mut svc = default_service();
		svc.ulimits.insert("nofile".to_string(), UlimitConfig::Pair { soft: 512, hard: 2048 });
		let ul = build_ulimits(&svc);
		assert_eq!(ul[0].soft, Some(512));
		assert_eq!(ul[0].hard, Some(2048));
	}
}
