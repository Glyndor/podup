use super::*;

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
