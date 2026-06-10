//! `extends:` directive — inheritance and field merging between service definitions.
//!
//! Services can extend another service within the same file or from an external
//! compose file referenced by path. Resolution is recursive (chains are supported)
//! and cycle detection uses a visited set to error early.
//!
//! Merge semantics: scalar fields from the child win; collection fields
//! (env vars, labels, vectors) are merged with the child taking precedence on
//! overlapping keys. See [`merge_service`] for full field-by-field rules.

use std::collections::HashSet;
use std::path::Path;

use super::parse_file_inner;
use super::types::{
	ComposeFile, DependsOn, EnvFile, EnvVars, Labels, Service, ServiceNetworks, StringOrList,
	Sysctls,
};
use crate::error::{ComposeError, Result};

const MAX_EXTENDS_DEPTH: usize = 16;

/// Resolve `extends:` only within the same file (no `file:` references).
///
/// Used by [`super::parse_str`] where there is no on-disk path.
pub(super) fn resolve_extends_same_file(file: &mut ComposeFile) -> Result<()> {
	let names: Vec<String> = file.services.keys().cloned().collect();
	for name in names {
		let mut visited: HashSet<String> = HashSet::new();
		resolve_one_extends_in_memory(file, &name, &mut visited, 0)?;
	}
	Ok(())
}

/// Resolve `extends:` for every service in `file`, including chains across
/// other compose files referenced by `extends.file`.
pub(super) fn resolve_all_extends(file: &mut ComposeFile, base_dir: &Path) -> Result<()> {
	let names: Vec<String> = file.services.keys().cloned().collect();
	for name in names {
		let mut visited: HashSet<String> = HashSet::new();
		resolve_one_extends(file, &name, base_dir, &mut visited, 0)?;
	}
	Ok(())
}

fn resolve_one_extends_in_memory(
	file: &mut ComposeFile,
	name: &str,
	visited: &mut HashSet<String>,
	depth: usize,
) -> Result<()> {
	if depth >= MAX_EXTENDS_DEPTH {
		return Err(ComposeError::Extends(format!(
			"extends chain exceeds maximum depth ({MAX_EXTENDS_DEPTH}) at service '{name}'"
		)));
	}
	if !visited.insert(name.to_string()) {
		return Err(ComposeError::Extends(format!("circular extends at {name}")));
	}

	let extends = match file.services.get(name).and_then(|s| s.extends.clone()) {
		Some(e) => e,
		None => return Ok(()),
	};

	if extends.file().is_some() {
		return Err(ComposeError::Extends(format!(
			"service '{name}' uses 'extends.file' but parser was given a string, not a path"
		)));
	}

	let base_name = extends.service().to_string();
	if base_name == name {
		return Err(ComposeError::Extends(format!(
			"service '{name}' extends itself"
		)));
	}

	if file.services.get(&base_name).is_none() {
		return Err(ComposeError::Extends(format!(
			"service '{name}' extends unknown service '{base_name}'"
		)));
	}
	resolve_one_extends_in_memory(file, &base_name, visited, depth + 1)?;

	let base = file
		.services
		.get(&base_name)
		.cloned()
		.ok_or_else(|| ComposeError::Extends(base_name.clone()))?;

	if let Some(svc) = file.services.get_mut(name) {
		let merged = merge_service(base, svc.clone());
		*svc = merged;
		svc.extends = None;
	}

	Ok(())
}

fn resolve_one_extends(
	file: &mut ComposeFile,
	name: &str,
	base_dir: &Path,
	visited: &mut HashSet<String>,
	depth: usize,
) -> Result<()> {
	if depth >= MAX_EXTENDS_DEPTH {
		return Err(ComposeError::Extends(format!(
			"extends chain exceeds maximum depth ({MAX_EXTENDS_DEPTH}) at service '{name}'"
		)));
	}
	if !visited.insert(name.to_string()) {
		return Err(ComposeError::Extends(format!("circular extends at {name}")));
	}

	let extends = match file.services.get(name).and_then(|s| s.extends.clone()) {
		Some(e) => e,
		None => return Ok(()),
	};

	let base_name = extends.service().to_string();

	let base_service = if let Some(file_path) = extends.file() {
		if !is_safe_extends_path(file_path) {
			return Err(ComposeError::Extends(format!(
				"service '{name}' extends.file must be a relative path with no parent traversal: {file_path}"
			)));
		}
		let abs = base_dir.join(file_path);
		let abs = abs.canonicalize().unwrap_or(abs);
		let dir = abs
			.parent()
			.map(|p| p.to_path_buf())
			.unwrap_or_else(|| base_dir.to_path_buf());
		let mut other = parse_file_inner(&abs, &dir)?;
		let mut nested_visited: HashSet<String> = HashSet::new();
		resolve_one_extends(&mut other, &base_name, &dir, &mut nested_visited, depth + 1)?;
		other.services.swap_remove(&base_name).ok_or_else(|| {
			ComposeError::Extends(format!(
				"service '{base_name}' not found in {}",
				abs.display()
			))
		})?
	} else {
		if base_name == name {
			return Err(ComposeError::Extends(format!(
				"service '{name}' extends itself"
			)));
		}
		if !file.services.contains_key(&base_name) {
			return Err(ComposeError::Extends(format!(
				"service '{name}' extends unknown service '{base_name}'"
			)));
		}
		resolve_one_extends(file, &base_name, base_dir, visited, depth + 1)?;
		file.services
			.get(&base_name)
			.cloned()
			.ok_or_else(|| ComposeError::Extends(base_name.clone()))?
	};

	if let Some(svc) = file.services.get_mut(name) {
		let merged = merge_service(base_service, svc.clone());
		*svc = merged;
		svc.extends = None;
	}

	Ok(())
}

/// Security check: reject `extends.file` with absolute or parent-traversing paths.
///
/// Exposed for unit testing.
pub(crate) fn is_safe_extends_path(path: &str) -> bool {
	let fp = std::path::Path::new(path);
	if fp.is_absolute() {
		return false;
	}
	// On Windows, paths like "/etc/passwd" are not `is_absolute()` (no drive letter)
	// but still escape the project directory via the root separator.
	if fp.components().next() == Some(std::path::Component::RootDir) {
		return false;
	}
	if fp
		.components()
		.any(|c| c == std::path::Component::ParentDir)
	{
		return false;
	}
	true
}

pub(super) fn merge_service(base: Service, override_svc: Service) -> Service {
	fn opt<T>(o: Option<T>, b: Option<T>) -> Option<T> {
		o.or(b)
	}

	fn merge_envvars(base: EnvVars, over: EnvVars) -> EnvVars {
		if matches!(over, EnvVars::Empty) && !matches!(base, EnvVars::Empty) {
			return base;
		}
		if matches!(base, EnvVars::Empty) {
			return over;
		}
		let mut merged: indexmap::IndexMap<String, Option<serde_yaml::Value>> =
			indexmap::IndexMap::new();
		for (k, v) in base.to_map() {
			merged.insert(k, v.map(serde_yaml::Value::String));
		}
		for (k, v) in over.to_map() {
			merged.insert(k, v.map(serde_yaml::Value::String));
		}
		EnvVars::Map(merged)
	}

	fn merge_labels(base: Labels, over: Labels) -> Labels {
		if base.is_empty() && over.is_empty() {
			return Labels::Empty;
		}
		let mut map: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
		for (k, v) in base.to_map() {
			map.insert(k, v);
		}
		for (k, v) in over.to_map() {
			map.insert(k, v);
		}
		Labels::Map(map)
	}

	fn merge_vec<T: Clone>(base: Vec<T>, over: Vec<T>) -> Vec<T> {
		if over.is_empty() {
			base
		} else {
			over
		}
	}

	fn merge_sol(base: StringOrList, over: StringOrList) -> StringOrList {
		if over.is_empty() {
			base
		} else {
			over
		}
	}

	fn merge_env_file(base: EnvFile, over: EnvFile) -> EnvFile {
		if over.is_empty() {
			base
		} else {
			over
		}
	}

	Service {
		image: opt(override_svc.image, base.image),
		build: override_svc.build.or(base.build),
		extends: override_svc.extends.or(base.extends),
		command: override_svc.command.or(base.command),
		entrypoint: override_svc.entrypoint.or(base.entrypoint),
		ports: merge_vec(base.ports, override_svc.ports),
		expose: merge_vec(base.expose, override_svc.expose),
		environment: merge_envvars(base.environment, override_svc.environment),
		env_file: merge_env_file(base.env_file, override_svc.env_file),
		volumes: merge_vec(base.volumes, override_svc.volumes),
		tmpfs: merge_sol(base.tmpfs, override_svc.tmpfs),
		volumes_from: merge_vec(base.volumes_from, override_svc.volumes_from),
		configs: merge_vec(base.configs, override_svc.configs),
		secrets: merge_vec(base.secrets, override_svc.secrets),
		networks: if matches!(override_svc.networks, ServiceNetworks::Empty) {
			base.networks
		} else {
			override_svc.networks
		},
		hostname: override_svc.hostname.or(base.hostname),
		domainname: override_svc.domainname.or(base.domainname),
		mac_address: override_svc.mac_address.or(base.mac_address),
		links: merge_vec(base.links, override_svc.links),
		external_links: merge_vec(base.external_links, override_svc.external_links),
		extra_hosts: merge_vec(base.extra_hosts, override_svc.extra_hosts),
		dns: merge_sol(base.dns, override_svc.dns),
		dns_search: merge_sol(base.dns_search, override_svc.dns_search),
		dns_opt: merge_sol(base.dns_opt, override_svc.dns_opt),
		network_mode: override_svc.network_mode.or(base.network_mode),
		depends_on: if matches!(override_svc.depends_on, DependsOn::Empty) {
			base.depends_on
		} else {
			override_svc.depends_on
		},
		healthcheck: override_svc.healthcheck.or(base.healthcheck),
		restart: override_svc.restart.or(base.restart),
		stop_signal: override_svc.stop_signal.or(base.stop_signal),
		stop_grace_period: override_svc.stop_grace_period.or(base.stop_grace_period),
		profiles: merge_vec(base.profiles, override_svc.profiles),
		post_start: merge_vec(base.post_start, override_svc.post_start),
		pre_stop: merge_vec(base.pre_stop, override_svc.pre_stop),
		labels: merge_labels(base.labels, override_svc.labels),
		annotations: merge_labels(base.annotations, override_svc.annotations),
		container_name: override_svc.container_name.or(base.container_name),
		user: override_svc.user.or(base.user),
		working_dir: override_svc.working_dir.or(base.working_dir),
		group_add: merge_vec(base.group_add, override_svc.group_add),
		platform: override_svc.platform.or(base.platform),
		cap_add: merge_vec(base.cap_add, override_svc.cap_add),
		cap_drop: merge_vec(base.cap_drop, override_svc.cap_drop),
		security_opt: merge_vec(base.security_opt, override_svc.security_opt),
		read_only: override_svc.read_only.or(base.read_only),
		privileged: override_svc.privileged.or(base.privileged),
		init: override_svc.init.or(base.init),
		tty: override_svc.tty.or(base.tty),
		stdin_open: override_svc.stdin_open.or(base.stdin_open),
		runtime: override_svc.runtime.or(base.runtime),
		shm_size: override_svc.shm_size.or(base.shm_size),
		userns_mode: override_svc.userns_mode.or(base.userns_mode),
		pid: override_svc.pid.or(base.pid),
		ipc: override_svc.ipc.or(base.ipc),
		uts: override_svc.uts.or(base.uts),
		cgroup_parent: override_svc.cgroup_parent.or(base.cgroup_parent),
		cgroup: override_svc.cgroup.or(base.cgroup),
		devices: merge_vec(base.devices, override_svc.devices),
		device_cgroup_rules: merge_vec(base.device_cgroup_rules, override_svc.device_cgroup_rules),
		storage_opt: {
			let mut m = base.storage_opt;
			for (k, v) in override_svc.storage_opt {
				m.insert(k, v);
			}
			m
		},
		scale: override_svc.scale.or(base.scale),
		cpu_shares: override_svc.cpu_shares.or(base.cpu_shares),
		cpu_quota: override_svc.cpu_quota.or(base.cpu_quota),
		cpu_period: override_svc.cpu_period.or(base.cpu_period),
		cpuset: override_svc.cpuset.or(base.cpuset),
		cpus: override_svc.cpus.or(base.cpus),
		cpu_count: override_svc.cpu_count.or(base.cpu_count),
		cpu_percent: override_svc.cpu_percent.or(base.cpu_percent),
		cpu_rt_runtime: override_svc.cpu_rt_runtime.or(base.cpu_rt_runtime),
		cpu_rt_period: override_svc.cpu_rt_period.or(base.cpu_rt_period),
		mem_limit: override_svc.mem_limit.or(base.mem_limit),
		memswap_limit: override_svc.memswap_limit.or(base.memswap_limit),
		mem_reservation: override_svc.mem_reservation.or(base.mem_reservation),
		mem_swappiness: override_svc.mem_swappiness.or(base.mem_swappiness),
		pids_limit: override_svc.pids_limit.or(base.pids_limit),
		oom_kill_disable: override_svc.oom_kill_disable.or(base.oom_kill_disable),
		oom_score_adj: override_svc.oom_score_adj.or(base.oom_score_adj),
		blkio_config: override_svc.blkio_config.or(base.blkio_config),
		logging: override_svc.logging.or(base.logging),
		sysctls: if matches!(override_svc.sysctls, Sysctls::Empty) {
			base.sysctls
		} else {
			override_svc.sysctls
		},
		ulimits: {
			let mut m = base.ulimits;
			for (k, v) in override_svc.ulimits {
				m.insert(k, v);
			}
			m
		},
		label_file: merge_sol(base.label_file, override_svc.label_file),
		attach: override_svc.attach.or(base.attach),
		pull_policy: override_svc.pull_policy.or(base.pull_policy),
		deploy: override_svc.deploy.or(base.deploy),
		develop: override_svc.develop.or(base.develop),
		gpus: override_svc.gpus.or(base.gpus),
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::compose::types::{ComposeFile, EnvVars, Labels, Service};
	use indexmap::IndexMap;

	fn svc(image: &str) -> Service {
		Service {
			image: Some(image.to_string()),
			..Default::default()
		}
	}

	// is_safe_extends_path

	#[test]
	fn safe_path_relative() {
		assert!(is_safe_extends_path("other.yml"));
	}

	#[test]
	fn safe_path_subdirectory() {
		assert!(is_safe_extends_path("bases/db.yml"));
	}

	#[test]
	fn unsafe_path_absolute() {
		assert!(!is_safe_extends_path("/etc/compose.yml"));
	}

	#[test]
	fn unsafe_path_parent_traversal() {
		assert!(!is_safe_extends_path("../secret.yml"));
	}

	#[test]
	fn unsafe_path_nested_traversal() {
		assert!(!is_safe_extends_path("a/../../etc/passwd"));
	}

	// merge_service — scalar fields

	#[test]
	fn merge_override_image_wins() {
		let base = svc("nginx:1.24");
		let over = svc("nginx:1.25");
		let merged = merge_service(base, over);
		assert_eq!(merged.image.as_deref(), Some("nginx:1.25"));
	}

	#[test]
	fn merge_base_image_used_when_override_missing() {
		let base = svc("nginx:1.24");
		let over = Service::default();
		let merged = merge_service(base, over);
		assert_eq!(merged.image.as_deref(), Some("nginx:1.24"));
	}

	// merge_service — env vars (override wins per key)

	#[test]
	fn merge_env_vars_override_wins_on_conflict() {
		let mut base_map = IndexMap::new();
		base_map.insert(
			"PORT".to_string(),
			Some(serde_yaml::Value::String("8080".into())),
		);
		let base = Service {
			environment: EnvVars::Map(base_map),
			..Default::default()
		};

		let mut over_map = IndexMap::new();
		over_map.insert(
			"PORT".to_string(),
			Some(serde_yaml::Value::String("9090".into())),
		);
		let over = Service {
			environment: EnvVars::Map(over_map),
			..Default::default()
		};

		let merged = merge_service(base, over);
		let env = merged.environment.to_map();
		assert_eq!(env.get("PORT").and_then(|v| v.as_deref()), Some("9090"));
	}

	#[test]
	fn merge_env_vars_base_key_preserved_when_not_overridden() {
		let mut base_map = IndexMap::new();
		base_map.insert(
			"BASE_ONLY".to_string(),
			Some(serde_yaml::Value::String("yes".into())),
		);
		let base = Service {
			environment: EnvVars::Map(base_map),
			..Default::default()
		};
		let over = Service::default();
		let merged = merge_service(base, over);
		let env = merged.environment.to_map();
		assert_eq!(env.get("BASE_ONLY").and_then(|v| v.as_deref()), Some("yes"));
	}

	// merge_service — labels (merged, override wins on conflict)

	#[test]
	fn merge_labels_both_preserved() {
		let mut base_im = IndexMap::new();
		base_im.insert("team".to_string(), "infra".to_string());
		let base = Service {
			labels: Labels::Map(base_im),
			..Default::default()
		};

		let mut over_im = IndexMap::new();
		over_im.insert("env".to_string(), "prod".to_string());
		let over = Service {
			labels: Labels::Map(over_im),
			..Default::default()
		};

		let merged = merge_service(base, over);
		let lm = merged.labels.to_map();
		assert_eq!(lm.get("team").map(|s| s.as_str()), Some("infra"));
		assert_eq!(lm.get("env").map(|s| s.as_str()), Some("prod"));
	}

	// resolve_extends_same_file — cycle detection

	#[test]
	fn cycle_detection_returns_error() {
		use crate::compose::types::ExtendsConfig;
		let mut file = ComposeFile::default();
		file.services.insert(
			"a".to_string(),
			Service {
				extends: Some(ExtendsConfig::Service("b".to_string())),
				..Default::default()
			},
		);
		file.services.insert(
			"b".to_string(),
			Service {
				extends: Some(ExtendsConfig::Service("a".to_string())),
				..Default::default()
			},
		);
		assert!(resolve_extends_same_file(&mut file).is_err());
	}

	#[test]
	fn self_extends_returns_error() {
		use crate::compose::types::ExtendsConfig;
		let mut file = ComposeFile::default();
		file.services.insert(
			"web".to_string(),
			Service {
				extends: Some(ExtendsConfig::Service("web".to_string())),
				..Default::default()
			},
		);
		assert!(resolve_extends_same_file(&mut file).is_err());
	}

	#[test]
	fn extends_unknown_service_returns_error() {
		use crate::compose::types::ExtendsConfig;
		let mut file = ComposeFile::default();
		file.services.insert(
			"web".to_string(),
			Service {
				extends: Some(ExtendsConfig::Service("nonexistent".to_string())),
				..Default::default()
			},
		);
		assert!(resolve_extends_same_file(&mut file).is_err());
	}

	#[test]
	fn extends_inherits_image_from_base() {
		use crate::compose::types::ExtendsConfig;
		let mut file = ComposeFile::default();
		file.services.insert("base".to_string(), svc("postgres:16"));
		file.services.insert(
			"db".to_string(),
			Service {
				extends: Some(ExtendsConfig::Service("base".to_string())),
				..Default::default()
			},
		);
		resolve_extends_same_file(&mut file).unwrap();
		assert_eq!(file.services["db"].image.as_deref(), Some("postgres:16"));
		assert!(file.services["db"].extends.is_none());
	}
}
