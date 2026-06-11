//! Pure configuration builders for container creation: restart policy, logging,
//! healthcheck, resource limits, and ulimits.
//!
//! Device, blkio, tmpfs, and label-file helpers live in [`super::container_misc`].

use crate::compose::types::{
	Command as ComposeCommand, HealthCheck, LoggingConfig, RestartPolicy as ComposeRestart, Service,
};
use crate::libpod::types::container::{HealthConfig, LinuxResources, LogConfig, Ulimit};
use crate::size;

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
		let name = match drp.condition.as_deref().unwrap_or("any") {
			"none" => "no",
			"on-failure" => "on-failure",
			_ => "unless-stopped",
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

pub(super) fn build_resource_limits(service: &Service) -> Option<LinuxResources> {
	use crate::libpod::types::container::{LinuxCPU, LinuxMemory, LinuxPids};

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
	let cpu_period = service.cpu_period;
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
				// GPU/device reservations are forwarded separately as CDI
				// devices; see [`cdi_devices`].
			}
		}
	}

	// nano_cpus (Docker) = cpus * 1e9; convert to OCI quota with 100ms period.
	let derived_cpu_quota = nano_cpus.map(|n| n / 10_000);
	let effective_cpu_quota = cpu_quota.or(derived_cpu_quota);
	let effective_cpu_period = if effective_cpu_quota.is_some() && cpu_period.is_none() {
		Some(100_000u64)
	} else {
		cpu_period
	};

	let has_memory = memory.is_some()
		|| mem_reservation.is_some()
		|| memswap.is_some()
		|| service.mem_swappiness.is_some()
		|| service.oom_kill_disable.is_some();

	let has_cpu = effective_cpu_quota.is_some()
		|| effective_cpu_period.is_some()
		|| service.cpu_shares.is_some()
		|| service.cpuset.is_some()
		|| service.cpu_rt_period.is_some()
		|| service.cpu_rt_runtime.is_some();

	let has_pids = pids_limit.is_some();

	if !has_memory && !has_cpu && !has_pids {
		return None;
	}

	let mem = if has_memory {
		Some(LinuxMemory {
			limit: memory,
			reservation: mem_reservation,
			swap: memswap,
			swappiness: service.mem_swappiness.map(|v| v as u64),
			disable_oom_killer: service.oom_kill_disable,
		})
	} else {
		None
	};

	let cpu = if has_cpu {
		Some(LinuxCPU {
			shares: service.cpu_shares,
			quota: effective_cpu_quota,
			period: effective_cpu_period,
			realtime_period: service.cpu_rt_period.map(|v| v as u64),
			realtime_runtime: service.cpu_rt_runtime,
			cpus: service.cpuset.clone(),
		})
	} else {
		None
	};

	let pids = pids_limit.map(|limit| LinuxPids { limit });

	Some(LinuxResources {
		memory: mem,
		cpu,
		pids,
		block_io: None,
		devices: vec![],
	})
}

pub(super) fn build_ulimits(service: &Service) -> Vec<Ulimit> {
	service
		.ulimits
		.iter()
		.map(|(name, cfg)| Ulimit {
			ulimit_type: name.clone(),
			soft: cfg.soft() as u64,
			hard: cfg.hard() as u64,
		})
		.collect()
}

/// Map `deploy.resources.reservations.devices` GPU reservations to Podman CDI
/// device names (e.g. `nvidia.com/gpu=all`).
///
/// Only NVIDIA GPU reservations are translated — the common case Podman exposes
/// through CDI. Reservations for other drivers or capabilities are warned about
/// and skipped, since there is no portable mapping for them.
pub(super) fn cdi_devices(service: &Service) -> Vec<String> {
	let mut out = Vec::new();

	let Some(reservations) = service
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.reservations.as_ref())
	else {
		return out;
	};

	for dev in &reservations.devices {
		let is_gpu = dev.capabilities.iter().any(|c| c == "gpu" || c == "nvidia");
		let driver = dev.driver.as_deref().unwrap_or("nvidia");
		if !is_gpu || (driver != "nvidia" && !driver.is_empty()) {
			tracing::warn!(
				"device reservation (driver {:?}, capabilities {:?}) is not supported and is ignored",
				dev.driver,
				dev.capabilities
			);
			continue;
		}

		if !dev.device_ids.is_empty() {
			for id in &dev.device_ids {
				out.push(format!("nvidia.com/gpu={id}"));
			}
			continue;
		}

		match dev.count.as_ref().map(|c| c.to_i64()) {
			// `count: all` (-1) or unspecified → request every GPU.
			Some(-1) | None => out.push("nvidia.com/gpu=all".to_string()),
			Some(n) if n > 0 => {
				for i in 0..n {
					out.push(format!("nvidia.com/gpu={i}"));
				}
			}
			Some(_) => {}
		}
	}

	out
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

	// --- resource limits ---

	#[test]
	fn build_resource_limits_empty_service() {
		assert!(build_resource_limits(&default_service()).is_none());
	}

	#[test]
	fn build_resource_limits_mem_limit() {
		let mut svc = default_service();
		svc.mem_limit = Some("512m".into());
		let res = build_resource_limits(&svc).unwrap();
		assert_eq!(res.memory.unwrap().limit, Some(512 * 1024 * 1024));
	}

	#[test]
	fn build_resource_limits_deploy_overrides() {
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
		let res = build_resource_limits(&svc).unwrap();
		assert_eq!(res.memory.unwrap().limit, Some(256 * 1024 * 1024));
	}

	#[test]
	fn build_resource_limits_cpus_converts_to_quota() {
		let mut svc = default_service();
		svc.cpus = Some("0.5".into());
		let res = build_resource_limits(&svc).unwrap();
		let cpu = res.cpu.unwrap();
		// 0.5 CPUs → 500_000_000 nano_cpus → quota = 50_000 (50ms per 100ms period)
		assert_eq!(cpu.quota, Some(50_000));
		assert_eq!(cpu.period, Some(100_000));
	}

	// --- ulimits ---

	#[test]
	fn build_ulimits_single_value() {
		use crate::compose::types::UlimitConfig;
		let mut svc = default_service();
		svc.ulimits
			.insert("nofile".to_string(), UlimitConfig::Single(1024));
		let ul = build_ulimits(&svc);
		assert_eq!(ul.len(), 1);
		assert_eq!(ul[0].ulimit_type, "nofile");
		assert_eq!(ul[0].soft, 1024);
		assert_eq!(ul[0].hard, 1024);
	}

	#[test]
	fn build_ulimits_pair() {
		use crate::compose::types::UlimitConfig;
		let mut svc = default_service();
		svc.ulimits.insert(
			"nofile".to_string(),
			UlimitConfig::Pair {
				soft: 512,
				hard: 2048,
			},
		);
		let ul = build_ulimits(&svc);
		assert_eq!(ul[0].soft, 512);
		assert_eq!(ul[0].hard, 2048);
	}

	// --- cdi devices ---

	fn cdi_for(yaml: &str) -> Vec<String> {
		let file = crate::parse_str(yaml).unwrap();
		cdi_devices(&file.services["app"])
	}

	#[test]
	fn cdi_gpu_count_all() {
		let got = cdi_for(
			"services:\n  app:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [gpu]\n              count: all\n",
		);
		assert_eq!(got, vec!["nvidia.com/gpu=all"]);
	}

	#[test]
	fn cdi_gpu_count_n_enumerates() {
		let got = cdi_for(
			"services:\n  app:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [gpu]\n              count: 2\n",
		);
		assert_eq!(got, vec!["nvidia.com/gpu=0", "nvidia.com/gpu=1"]);
	}

	#[test]
	fn cdi_gpu_device_ids() {
		let got = cdi_for(
			"services:\n  app:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [gpu]\n              device_ids: [\"GPU-abc\", \"1\"]\n",
		);
		assert_eq!(got, vec!["nvidia.com/gpu=GPU-abc", "nvidia.com/gpu=1"]);
	}

	#[test]
	fn cdi_non_gpu_skipped() {
		let got = cdi_for(
			"services:\n  app:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [tpu]\n              driver: google\n",
		);
		assert!(got.is_empty());
	}

	#[test]
	fn cdi_absent_without_deploy() {
		assert!(cdi_devices(&default_service()).is_empty());
	}
}
