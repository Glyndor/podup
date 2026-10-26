use super::*;

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
fn effective_memory_limit_forwards_negative_one() {
	// `parse_memory("-1")` returns `Some(-1)`. The helper forwards the
	// engine's value verbatim: `build_resource_limits` puts `-1` into
	// `LinuxMemory.limit` and lets Podman interpret it as "no cap" at
	// the API layer. Filtering here would change what the engine
	// sends. The audit applies the "not a limit" filter on top of the
	// helper (`#1894`).
	let mut svc = default_service();
	svc.mem_limit = Some("-1".into());
	assert_eq!(effective_memory_limit(&svc), Some(-1));
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
fn effective_cpu_quota_forwards_negative_one() {
	// The Docker API treats `cpu_quota: -1` as "unlimited", and the
	// engine forwards it verbatim into `LinuxCPU.quota`. The helper
	// must match that so the engine and the audit's `cpu_limit_in_effect`
	// (which applies the "<= 0 is not a limit" filter) agree on
	// what the OCI spec carries (`#1894`).
	let mut svc = default_service();
	svc.cpu_quota = Some(-1);
	assert_eq!(effective_cpu_quota(&svc), Some(-1));
}

#[test]
fn effective_cpu_quota_forwards_zero() {
	// `cpu_quota: 0` is forwarded verbatim: the engine builds
	// `LinuxCPU { quota: 0, ... }` and lets libpod reject it if the
	// operator really meant a zero-time container. The audit treats
	// `0` as "no limit" via [`cpu_limit_in_effect`] (`#1894`).
	let mut svc = default_service();
	svc.cpu_quota = Some(0);
	assert_eq!(effective_cpu_quota(&svc), Some(0));
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
