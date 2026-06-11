//! Podman libpod container API request and response types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// SpecGenerator — container create request
// ---------------------------------------------------------------------------

/// Full container specification sent to `POST /libpod/containers/create`.
#[derive(Serialize, Default)]
pub struct SpecGenerator {
	pub name: String,
	pub image: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub command: Option<Vec<String>>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub entrypoint: Option<Vec<String>>,

	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub env: HashMap<String, String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub terminal: Option<bool>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub stdin: Option<bool>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub user: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub work_dir: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_signal: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_timeout: Option<u64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub hostname: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub domainname: Option<String>,

	// Labels and annotations
	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub labels: HashMap<String, String>,

	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub annotations: HashMap<String, String>,

	// Capabilities and security
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub cap_add: Vec<String>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub cap_drop: Vec<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub privileged: Option<bool>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub read_only_filesystem: Option<bool>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub security_opt: Vec<String>,

	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub sysctl: HashMap<String, String>,

	// Networking
	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub expose: HashMap<u16, String>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub portmappings: Vec<PortMapping>,

	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub networks: HashMap<String, PerNetworkOptions>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub netns: Option<Namespace>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub extra_hosts: Vec<String>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub dns_server: Vec<String>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub dns_search: Vec<String>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub dns_option: Vec<String>,

	// Volumes and mounts
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub mounts: Vec<Mount>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub volumes_from: Vec<String>,

	// Namespace modes
	#[serde(skip_serializing_if = "Option::is_none")]
	pub userns: Option<Namespace>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub pidns: Option<Namespace>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub ipcns: Option<Namespace>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub utsns: Option<Namespace>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cgroupns: Option<Namespace>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cgroup_parent: Option<String>,

	// Resource limits
	#[serde(skip_serializing_if = "Option::is_none")]
	pub resource_limits: Option<LinuxResources>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub ulimits: Vec<Ulimit>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub shm_size: Option<i64>,

	// Healthcheck
	#[serde(skip_serializing_if = "Option::is_none")]
	pub healthconfig: Option<HealthConfig>,

	// Logging
	#[serde(skip_serializing_if = "Option::is_none")]
	pub log_configuration: Option<LogConfig>,

	// Init process
	#[serde(skip_serializing_if = "Option::is_none")]
	pub init: Option<bool>,

	// Restart policy
	#[serde(skip_serializing_if = "Option::is_none")]
	pub restart_policy: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub restart_tries: Option<u64>,

	// Devices
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub devices: Vec<LinuxDevice>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub device_cgroup_rule: Vec<String>,

	// Groups
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub groups: Vec<String>,

	// OOM
	#[serde(skip_serializing_if = "Option::is_none")]
	pub oom_score_adj: Option<i64>,

	// Runtime
	#[serde(skip_serializing_if = "Option::is_none")]
	pub runtime: Option<String>,

	// Links (deprecated in rootless, accepted but may be ignored)
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub links: Vec<String>,

	// Platform selection
	#[serde(skip_serializing_if = "Option::is_none")]
	pub image_arch: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub image_os: Option<String>,

	// Volume image handling
	#[serde(skip_serializing_if = "Option::is_none")]
	pub image_volume_mode: Option<String>,

	// Storage options
	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub storage_opts: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Nested types
// ---------------------------------------------------------------------------

/// Port mapping for SpecGenerator.
#[derive(Serialize, Default)]
pub struct PortMapping {
	pub container_port: u16,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub host_port: Option<u16>,

	#[serde(skip_serializing_if = "String::is_empty", default)]
	pub host_ip: String,

	pub protocol: String,

	/// Number of ports to map starting from `container_port` (range mapping).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub range: Option<u16>,
}

/// Per-network connection options (for SpecGenerator `networks` map).
#[derive(Serialize, Default)]
pub struct PerNetworkOptions {
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub aliases: Vec<String>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub static_ips: Vec<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub static_mac: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub interface_name: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver_opts: Option<HashMap<String, String>>,
}

/// Linux network/pid/ipc/uts/cgroup namespace specification.
#[derive(Serialize, Clone)]
pub struct Namespace {
	pub nsmode: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub value: Option<String>,
}

impl Namespace {
	pub fn new(mode: impl Into<String>) -> Self {
		Self { nsmode: mode.into(), value: None }
	}

	pub fn container(id: impl Into<String>) -> Self {
		Self { nsmode: "container".into(), value: Some(id.into()) }
	}
}

/// OCI mount specification for SpecGenerator.
#[derive(Serialize, Default)]
pub struct Mount {
	#[serde(rename = "type")]
	pub mount_type: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub source: Option<String>,

	pub destination: String,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub options: Vec<String>,
}

/// Linux OCI resource limits for SpecGenerator.
#[derive(Serialize, Default)]
pub struct LinuxResources {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub memory: Option<LinuxMemory>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpu: Option<LinuxCPU>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub pids: Option<LinuxPids>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub block_io: Option<LinuxBlockIO>,

	/// GPU device access rules (maps `deploy.resources.reservations.devices`).
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub devices: Vec<LinuxDeviceCgroup>,
}

/// Linux memory resource limits.
#[derive(Serialize, Default)]
pub struct LinuxMemory {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub limit: Option<i64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub reservation: Option<i64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub swap: Option<i64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub swappiness: Option<u64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub disable_oom_killer: Option<bool>,
}

/// Linux CPU resource limits.
#[derive(Serialize, Default)]
pub struct LinuxCPU {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub shares: Option<u64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub quota: Option<i64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub period: Option<u64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub realtime_period: Option<u64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub realtime_runtime: Option<i64>,

	/// CPU affinity as a cpuset string (e.g. `"0-3,5"`).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpus: Option<String>,
}

/// Linux pids (process count) limit.
#[derive(Serialize)]
pub struct LinuxPids {
	pub limit: i64,
}

/// Linux block I/O resource limits.
#[derive(Serialize, Default)]
pub struct LinuxBlockIO {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub weight: Option<u16>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub weight_device: Vec<LinuxWeightDevice>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub throttle_read_bps_device: Vec<LinuxThrottleDevice>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub throttle_write_bps_device: Vec<LinuxThrottleDevice>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub throttle_read_iops_device: Vec<LinuxThrottleDevice>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub throttle_write_iops_device: Vec<LinuxThrottleDevice>,
}

/// Block device weight entry.
#[derive(Serialize)]
pub struct LinuxWeightDevice {
	pub major: i64,
	pub minor: i64,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub weight: Option<u16>,
}

/// Block device I/O throttle entry.
#[derive(Serialize)]
pub struct LinuxThrottleDevice {
	pub major: i64,
	pub minor: i64,
	pub rate: u64,
}

/// cgroup device access rule (for GPU access via `deploy.resources`).
#[derive(Serialize)]
pub struct LinuxDeviceCgroup {
	pub allow: bool,

	#[serde(rename = "type", skip_serializing_if = "Option::is_none")]
	pub device_type: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub major: Option<i64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub minor: Option<i64>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub access: Option<String>,
}

/// Process rlimit entry.
#[derive(Serialize)]
pub struct Ulimit {
	#[serde(rename = "type")]
	pub ulimit_type: String,
	pub soft: u64,
	pub hard: u64,
}

/// Container healthcheck configuration (same structure as Docker).
#[derive(Serialize, Default)]
pub struct HealthConfig {
	#[serde(rename = "Test", skip_serializing_if = "Option::is_none")]
	pub test: Option<Vec<String>>,

	#[serde(rename = "Interval", skip_serializing_if = "Option::is_none")]
	pub interval: Option<i64>,

	#[serde(rename = "Timeout", skip_serializing_if = "Option::is_none")]
	pub timeout: Option<i64>,

	#[serde(rename = "Retries", skip_serializing_if = "Option::is_none")]
	pub retries: Option<i64>,

	#[serde(rename = "StartPeriod", skip_serializing_if = "Option::is_none")]
	pub start_period: Option<i64>,

	#[serde(rename = "StartInterval", skip_serializing_if = "Option::is_none")]
	pub start_interval: Option<i64>,
}

/// Container log driver configuration.
#[derive(Serialize)]
pub struct LogConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,

	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub options: HashMap<String, String>,
}

/// Linux OCI device specification.
#[derive(Serialize)]
pub struct LinuxDevice {
	pub path: String,

	#[serde(rename = "type")]
	pub device_type: String,

	pub major: i64,
	pub minor: i64,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub file_mode: Option<u32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub uid: Option<u32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub gid: Option<u32>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Response from `POST /libpod/containers/create`.
#[derive(Deserialize)]
pub struct CreateResponse {
	#[serde(rename = "Id")]
	pub id: String,
}

/// Entry in the `GET /libpod/containers/json` response array.
#[derive(Deserialize)]
pub struct ContainerListEntry {
	#[serde(rename = "Id")]
	pub id: String,

	#[serde(rename = "Names", default)]
	pub names: Vec<String>,

	#[serde(rename = "Image", default)]
	pub image: String,

	#[serde(rename = "Status", default)]
	pub status: String,

	#[serde(rename = "Ports", default)]
	pub ports: Vec<ContainerPort>,
}

/// Port mapping entry in container list response.
#[derive(Deserialize, Default)]
pub struct ContainerPort {
	pub host_ip: Option<String>,
	pub host_port: Option<u16>,
	pub container_port: u16,
	pub protocol: Option<String>,
}

/// Response from `GET /libpod/containers/{name}/json`.
#[derive(Deserialize, Default)]
pub struct ContainerInspect {
	#[serde(rename = "State")]
	pub state: Option<ContainerState>,

	#[serde(rename = "NetworkSettings")]
	pub network_settings: Option<NetworkSettings>,
}

/// Container state sub-object.
#[derive(Deserialize, Default)]
pub struct ContainerState {
	#[serde(rename = "Status")]
	pub status: Option<String>,

	#[serde(rename = "ExitCode")]
	pub exit_code: Option<i64>,

	#[serde(rename = "Health")]
	pub health: Option<HealthState>,
}

/// Container health state sub-object.
#[derive(Deserialize)]
pub struct HealthState {
	#[serde(rename = "Status")]
	pub status: Option<String>,
}

/// Network settings sub-object from container inspect.
#[derive(Deserialize, Default)]
pub struct NetworkSettings {
	#[serde(rename = "Ports", default)]
	pub ports: HashMap<String, Option<Vec<HostBinding>>>,
}

/// Host port binding from container inspect network settings.
#[derive(Deserialize, Clone)]
pub struct HostBinding {
	#[serde(rename = "HostIp")]
	pub host_ip: Option<String>,

	#[serde(rename = "HostPort")]
	pub host_port: Option<String>,
}

/// Response from `POST /libpod/containers/{name}/wait`.
#[derive(Deserialize, Default)]
pub struct WaitResponse {
	#[serde(rename = "StatusCode", default)]
	pub status_code: i64,

	#[serde(rename = "Error")]
	pub error: Option<WaitError>,
}

/// Error sub-object in wait response.
#[derive(Deserialize)]
pub struct WaitError {
	#[serde(rename = "Message")]
	pub message: Option<String>,
}

/// Response from `GET /libpod/containers/{name}/top`.
#[derive(Deserialize, Default)]
pub struct TopResponse {
	#[serde(rename = "Titles")]
	pub titles: Option<Vec<String>>,

	#[serde(rename = "Processes")]
	pub processes: Option<Vec<Vec<String>>>,
}
