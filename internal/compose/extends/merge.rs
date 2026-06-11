//! Field-by-field merge of a base service into an overriding service.
//!
//! Scalar fields take the override when present, else the base. Collection
//! fields (env vars, labels, maps) are merged with the override winning on
//! overlapping keys; list-shaped fields take the override wholesale when
//! non-empty, otherwise fall back to the base.

use super::super::types::{
	DependsOn, EnvFile, EnvVars, Labels, Service, ServiceNetworks, StringOrList, Sysctls,
};

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
