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
fn build_resource_limits_deploy_cpus_pids_and_reservation() {
	// With no top-level cpus/pids/mem_reservation, the deploy block supplies
	// them: limits.cpus → quota, limits.pids → pids limit, reservations.memory
	// → memory soft limit.
	use crate::compose::types::{DeployConfig, ResourceSpec, ResourcesConfig};
	let mut svc = default_service();
	svc.deploy = Some(DeployConfig {
		resources: Some(ResourcesConfig {
			limits: Some(ResourceSpec {
				cpus: Some("2".into()),
				pids: Some(512),
				..Default::default()
			}),
			reservations: Some(ResourceSpec {
				memory: Some("128m".into()),
				..Default::default()
			}),
		}),
		..Default::default()
	});
	let res = build_resource_limits(&svc).unwrap();
	// 2 CPUs → 2e9 nano_cpus → quota = 200_000.
	assert_eq!(res.cpu.unwrap().quota, Some(200_000));
	assert_eq!(res.pids.unwrap().limit, 512);
	assert_eq!(res.memory.unwrap().reservation, Some(128 * 1024 * 1024));
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

// --- effective-limit helpers (shared with the audit, #1894) ---

#[test]
fn effective_memory_limit_top_level_wins_over_deploy() {
	use crate::compose::types::{DeployConfig, ResourceSpec, ResourcesConfig};
	let mut svc = default_service();
	svc.mem_limit = Some("256m".into());
	svc.deploy = Some(DeployConfig {
		resources: Some(ResourcesConfig {
			limits: Some(ResourceSpec {
				memory: Some("512m".into()),
				..Default::default()
			}),
			reservations: None,
		}),
		..Default::default()
	});
	// Top-level wins; the deploy block is ignored once the top level set one,
	// matching `build_resource_limits`. A regression to a `max`-based
	// resolution would silently pick 512m here.
	assert_eq!(effective_memory_limit(&svc), Some(256 * 1024 * 1024));
}

#[test]
fn effective_memory_limit_falls_back_to_deploy() {
	use crate::compose::types::{DeployConfig, ResourceSpec, ResourcesConfig};
	let mut svc = default_service();
	svc.deploy = Some(DeployConfig {
		resources: Some(ResourcesConfig {
			limits: Some(ResourceSpec {
				memory: Some("512m".into()),
				..Default::default()
			}),
			reservations: None,
		}),
		..Default::default()
	});
	assert_eq!(effective_memory_limit(&svc), Some(512 * 1024 * 1024));
}

#[test]
fn effective_memory_limit_filters_negative_one() {
	// `parse_memory("-1")` returns `Some(-1)`, the engine's "no cap"
	// sentinel for `memswap_limit:`. The helper must filter it out so
	// callers cannot mistake "no cap" for a cap.
	let mut svc = default_service();
	svc.mem_limit = Some("-1".into());
	assert_eq!(effective_memory_limit(&svc), None);
}

#[test]
fn effective_memory_limit_unparseable_is_no_limit() {
	let mut svc = default_service();
	svc.mem_limit = Some("not-a-size".into());
	assert_eq!(effective_memory_limit(&svc), None);
}

#[test]
fn effective_cpu_quota_top_level_cpu_quota_wins() {
	// Explicit `cpu_quota:` overrides any derived value.
	let mut svc = default_service();
	svc.cpu_quota = Some(25_000);
	svc.cpus = Some("2".into());
	assert_eq!(effective_cpu_quota(&svc), Some(25_000));
}

#[test]
fn effective_cpu_quota_derived_from_top_level_cpus() {
	let mut svc = default_service();
	svc.cpus = Some("0.5".into());
	// 0.5 CPUs parses to 500_000_000 nano_cpus; the helper divides by
	// 10_000 to yield a quota of 50_000 over the default 100ms period.
	assert_eq!(effective_cpu_quota(&svc), Some(50_000));
}

#[test]
fn effective_cpu_quota_top_level_cpus_wins_over_deploy() {
	use crate::compose::types::{DeployConfig, ResourceSpec, ResourcesConfig};
	let mut svc = default_service();
	svc.cpus = Some("0.5".into());
	svc.deploy = Some(DeployConfig {
		resources: Some(ResourcesConfig {
			limits: Some(ResourceSpec {
				cpus: Some("2".into()),
				..Default::default()
			}),
			reservations: None,
		}),
		..Default::default()
	});
	// Top-level wins; deploy is ignored. A regression to `max` would
	// wrongly pick the larger deploy value here.
	assert_eq!(effective_cpu_quota(&svc), Some(50_000));
}

#[test]
fn effective_cpu_quota_filters_negative_one() {
	// The Docker API treats `cpu_quota: -1` as "unlimited"; the helper
	// must filter it out so the audit's `no_cpu_limit` and the engine's
	// `build_resource_limits` agree.
	let mut svc = default_service();
	svc.cpu_quota = Some(-1);
	assert_eq!(effective_cpu_quota(&svc), None);
}

#[test]
fn effective_cpu_quota_filters_zero() {
	let mut svc = default_service();
	svc.cpu_quota = Some(0);
	assert_eq!(effective_cpu_quota(&svc), None);
}

#[test]
fn effective_cpu_quota_unparseable_cpus_is_no_limit() {
	let mut svc = default_service();
	svc.cpus = Some("nan".into());
	assert_eq!(effective_cpu_quota(&svc), None);
}

#[test]
fn effective_cpu_quota_falls_back_to_deploy() {
	use crate::compose::types::{DeployConfig, ResourceSpec, ResourcesConfig};
	let mut svc = default_service();
	svc.deploy = Some(DeployConfig {
		resources: Some(ResourcesConfig {
			limits: Some(ResourceSpec {
				cpus: Some("2".into()),
				..Default::default()
			}),
			reservations: None,
		}),
		..Default::default()
	});
	// 2 CPUs parses to 2e9 nano_cpus; divided by 10_000 the helper
	// yields a quota of 200_000.
	assert_eq!(effective_cpu_quota(&svc), Some(200_000));
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

#[test]
fn build_ulimits_clamps_soft_above_hard() {
	use crate::compose::types::UlimitConfig;
	let mut svc = default_service();
	svc.ulimits.insert(
		"nofile".to_string(),
		UlimitConfig::Pair {
			soft: 65535,
			hard: 1024,
		},
	);
	let ul = build_ulimits(&svc);
	assert_eq!(ul[0].soft, 1024, "soft must be clamped down to hard");
	assert_eq!(ul[0].hard, 1024);
}

#[test]
fn build_ulimits_rejects_unknown_resource_name() {
	use crate::compose::types::UlimitConfig;
	let mut svc = default_service();
	svc.ulimits
		.insert("bogus,inject=1".to_string(), UlimitConfig::Single(1024));
	assert!(
		build_ulimits(&svc).is_empty(),
		"an unknown ulimit name must be dropped, not forwarded"
	);
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
fn cdi_top_level_gpus_all() {
	assert_eq!(
		cdi_for("services:\n  app:\n    image: x\n    gpus: all\n"),
		vec!["nvidia.com/gpu=all"]
	);
}

#[test]
fn cdi_top_level_gpus_count() {
	assert_eq!(
		cdi_for("services:\n  app:\n    image: x\n    gpus: 2\n"),
		vec!["nvidia.com/gpu=0", "nvidia.com/gpu=1"]
	);
}

#[test]
fn cdi_top_level_gpus_device_list() {
	assert_eq!(
		cdi_for(
			"services:\n  app:\n    image: x\n    gpus:\n      - capabilities: [gpu]\n        device_ids: [\"GPU-xyz\"]\n",
		),
		vec!["nvidia.com/gpu=GPU-xyz"]
	);
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
