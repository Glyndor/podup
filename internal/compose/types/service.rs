//! [`Service`] struct — the central type representing a single compose service.
//!
//! Fields map 1-to-1 to the Docker Compose specification. Optional fields use
//! `Option<T>` so that absent keys are distinguishable from explicit nulls
//! during `extends:` merging.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::build::{BuildConfig, ExtendsConfig};
use super::deploy::DeployConfig;
use super::develop::DevelopConfig;
use super::network::ServiceNetworks;
use super::volume::{ServiceConfigRef, ServiceSecretRef, VolumeMount};
use super::{
	BlkioConfig, Command, DependsOn, EnvFile, EnvVars, GpuSpec, HealthCheck, Labels, LifecycleHook,
	LoggingConfig, PortMapping, RestartPolicy, StringOrList, Sysctls, UlimitConfig,
};

/// A single service definition.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Service {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub image: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub build: Option<BuildConfig>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub extends: Option<ExtendsConfig>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub command: Option<Command>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub entrypoint: Option<Command>,

	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub ports: Vec<PortMapping>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub expose: Vec<String>,

	#[serde(default)]
	pub environment: EnvVars,
	#[serde(default)]
	pub env_file: EnvFile,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub volumes: Vec<VolumeMount>,
	#[serde(default)]
	pub tmpfs: StringOrList,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub volumes_from: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub configs: Vec<ServiceConfigRef>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub secrets: Vec<ServiceSecretRef>,

	#[serde(default)]
	pub networks: ServiceNetworks,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub hostname: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub domainname: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mac_address: Option<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub links: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub external_links: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub extra_hosts: Vec<String>,
	#[serde(default)]
	pub dns: StringOrList,
	#[serde(default)]
	pub dns_search: StringOrList,
	#[serde(default)]
	pub dns_opt: StringOrList,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub network_mode: Option<String>,

	#[serde(default)]
	pub depends_on: DependsOn,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub healthcheck: Option<HealthCheck>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub restart: Option<RestartPolicy>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_signal: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_grace_period: Option<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub profiles: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub post_start: Vec<LifecycleHook>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub pre_stop: Vec<LifecycleHook>,

	#[serde(default)]
	pub labels: Labels,
	#[serde(default)]
	pub annotations: Labels,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub container_name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub user: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub working_dir: Option<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub group_add: Vec<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub platform: Option<String>,

	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub cap_add: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub cap_drop: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub security_opt: Vec<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub read_only: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub privileged: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub init: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tty: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stdin_open: Option<bool>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub runtime: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub shm_size: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub userns_mode: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub pid: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub ipc: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub uts: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cgroup_parent: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cgroup: Option<String>,

	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub devices: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub device_cgroup_rules: Vec<String>,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub storage_opt: HashMap<String, String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub scale: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpu_shares: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpu_quota: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpu_period: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpuset: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpus: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpu_count: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpu_percent: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpu_rt_runtime: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpu_rt_period: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mem_limit: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub memswap_limit: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mem_reservation: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mem_swappiness: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub pids_limit: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub oom_kill_disable: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub oom_score_adj: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub blkio_config: Option<BlkioConfig>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub logging: Option<LoggingConfig>,
	#[serde(default)]
	pub sysctls: Sysctls,
	#[serde(default, skip_serializing_if = "IndexMap::is_empty")]
	pub ulimits: IndexMap<String, UlimitConfig>,

	#[serde(default)]
	pub label_file: StringOrList,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub attach: Option<bool>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub pull_policy: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub deploy: Option<DeployConfig>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub develop: Option<DevelopConfig>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub gpus: Option<GpuSpec>,
}
