//! Resource-limit builders: CPU/memory/pids limits, ulimits, and CDI device
//! reservations.

use crate::compose::types::Service;
use crate::libpod::types::container::{LinuxResources, Ulimit};
use crate::size;

// ---------------------------------------------------------------------------
// Resource limits
// ---------------------------------------------------------------------------

pub(crate) fn build_resource_limits(service: &Service) -> Option<LinuxResources> {
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

pub(crate) fn build_ulimits(service: &Service) -> Vec<Ulimit> {
	service
		.ulimits
		.iter()
		.map(|(name, cfg)| Ulimit {
			ulimit_type: name.clone(),
			soft: ulimit_value(cfg.soft(), name, "soft"),
			hard: ulimit_value(cfg.hard(), name, "hard"),
		})
		.collect()
}

/// Convert a compose ulimit value to the libpod `u64`. `-1` is the conventional
/// "unlimited" sentinel (`u64::MAX`); any other negative value is invalid and
/// would otherwise wrap to a huge number via `as u64`, so it is rejected to `0`
/// with a warning instead of silently becoming an enormous limit.
fn ulimit_value(value: i64, name: &str, which: &str) -> u64 {
	match value {
		-1 => u64::MAX,
		v if v < 0 => {
			tracing::warn!(
				"ulimit {name} {which} value {v} is invalid (only -1 means unlimited); using 0"
			);
			0
		}
		v => v as u64,
	}
}

/// Upper bound on an explicit GPU `count:` — clamps an untrusted compose value
/// so a huge count cannot allocate billions of device-id strings (OOM). Far
/// above any real host's GPU count.
const MAX_GPU_DEVICES: i64 = 64;

/// Map `deploy.resources.reservations.devices` GPU reservations to Podman CDI
/// device names (e.g. `nvidia.com/gpu=all`).
///
/// Only NVIDIA GPU reservations are translated — the common case Podman exposes
/// through CDI. Reservations for other drivers or capabilities are warned about
/// and skipped, since there is no portable mapping for them.
pub(crate) fn cdi_devices(service: &Service) -> Vec<String> {
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
				// Clamp to a sane device count: the value comes from an untrusted
				// compose file and a huge `count:` would otherwise allocate billions
				// of device-id strings during spec assembly (OOM) before any
				// container is created. No host has anywhere near this many GPUs.
				if n > MAX_GPU_DEVICES {
					tracing::warn!(
						"device reservation count {n} exceeds the maximum of {MAX_GPU_DEVICES}; \
						 clamping (no host has this many GPUs)"
					);
				}
				for i in 0..n.min(MAX_GPU_DEVICES) {
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
	use crate::compose::types::Service;

	fn default_service() -> Service {
		Service::default()
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

	// --- ulimit value conversion ---

	#[test]
	fn ulimit_minus_one_is_unlimited() {
		assert_eq!(ulimit_value(-1, "nofile", "soft"), u64::MAX);
	}

	#[test]
	fn ulimit_other_negative_clamped_to_zero() {
		// Must not wrap to a huge u64 via `as`.
		assert_eq!(ulimit_value(-5, "nofile", "soft"), 0);
	}

	#[test]
	fn ulimit_positive_passes_through() {
		assert_eq!(ulimit_value(1024, "nofile", "hard"), 1024);
	}

	// --- gpu count clamp ---

	#[test]
	fn cdi_gpu_count_is_clamped() {
		let yaml = format!(
			"services:\n  g:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [gpu]\n              count: {}\n",
			MAX_GPU_DEVICES + 10_000
		);
		let file = crate::compose::parse_str(&yaml).unwrap();
		let out = cdi_devices(&file.services["g"]);
		assert_eq!(out.len(), MAX_GPU_DEVICES as usize);
	}
}
