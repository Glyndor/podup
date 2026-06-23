//! Nested request types referenced by [`SpecGenerator`](super::SpecGenerator).

use std::collections::HashMap;

use serde::Serialize;

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

/// A Podman-native secret attached to a container create spec, equivalent to
/// `podman run --secret`. Mirrors the libpod `Secret` type, which carries no
/// JSON tags upstream, so the wire keys are PascalCase (`Source`, `Target`, …).
/// `Source` names an existing Podman secret; `Target` is the mount destination
/// (a bare name lands under `/run/secrets/`, an absolute path is used as-is).
#[derive(Serialize, Default)]
pub struct Secret {
	#[serde(rename = "Source")]
	pub source: String,

	#[serde(rename = "Target", skip_serializing_if = "Option::is_none")]
	pub target: Option<String>,

	#[serde(rename = "UID", skip_serializing_if = "Option::is_none")]
	pub uid: Option<u32>,

	#[serde(rename = "GID", skip_serializing_if = "Option::is_none")]
	pub gid: Option<u32>,

	#[serde(rename = "Mode", skip_serializing_if = "Option::is_none")]
	pub mode: Option<u32>,
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
	/// Build a namespace with the given mode and no associated value.
	pub fn new(mode: impl Into<String>) -> Self {
		Self {
			nsmode: mode.into(),
			value: None,
		}
	}

	/// Build a `container:<id>` namespace sharing another container's namespace.
	pub fn container(id: impl Into<String>) -> Self {
		Self {
			nsmode: "container".into(),
			value: Some(id.into()),
		}
	}

	/// Parse a compose-style namespace string.
	///
	/// `"container:name"` → `{ nsmode: "container", value: "name" }`.
	/// Anything else → `{ nsmode: mode, value: None }`.
	pub fn parse(mode: impl Into<String>) -> Self {
		let mode = mode.into();
		if let Some(id) = mode.strip_prefix("container:") {
			Self::container(id)
		} else {
			Self::new(mode)
		}
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

/// Named volume attachment for SpecGenerator (goes in `volumes`, not `mounts`).
#[derive(Serialize, Default)]
pub struct NamedVolume {
	#[serde(rename = "Name")]
	pub name: String,

	#[serde(rename = "Dest")]
	pub dest: String,

	#[serde(rename = "Options", skip_serializing_if = "Vec::is_empty", default)]
	pub options: Vec<String>,

	/// Mount only this sub-directory of the volume (compose `volume.subpath`).
	#[serde(rename = "SubPath", skip_serializing_if = "Option::is_none", default)]
	pub sub_path: Option<String>,
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

/// Action Podman takes when a container's healthcheck transitions to unhealthy
/// (Podman 5's `--health-on-failure`).
///
/// Podman's `define.HealthCheckOnFailureAction` is an untagged `int` with no
/// custom JSON marshaller, so it travels the wire as a bare number. The explicit
/// discriminants below pin each variant to Podman's constant value; `Invalid`
/// (1) is Podman's error sentinel and is never emitted by a valid spec.
///
/// The variants are the complete Podman action set so the API surface is correct;
/// no compose key maps to them yet, hence `allow(dead_code)` until the
/// `--health-on-failure` plumbing lands.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum HealthCheckOnFailureAction {
	/// Take no action; only mark the container unhealthy (Podman's `none`, `0`).
	None = 0,
	/// Kill the container (Podman's `kill`, `2`).
	Kill = 2,
	/// Restart the container (Podman's `restart`, `3`).
	Restart = 3,
	/// Stop the container (Podman's `stop`, `4`).
	Stop = 4,
}

impl Serialize for HealthCheckOnFailureAction {
	fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		// Wire form is the bare integer Podman assigns each action.
		serializer.serialize_u8(*self as u8)
	}
}

/// Startup-healthcheck configuration (Podman 5's `--health-startup-*`).
///
/// Podman's `define.StartupHealthCheck` embeds `Schema2HealthConfig` and adds a
/// `Successes` count; the embedded probe fields are flattened to the top level of
/// the `startupHealthConfig` object with their PascalCase keys (`Test`,
/// `Interval`, …), which is exactly [`HealthConfig`]'s wire shape — so it is
/// reused here via `#[serde(flatten)]`.
#[derive(Serialize, Default)]
pub struct StartupHealthCheck {
	/// The probe definition (test command, interval, timeout, retries, …),
	/// flattened so its fields sit alongside `Successes`.
	#[serde(flatten)]
	pub health_config: HealthConfig,

	/// Number of consecutive successes required before the container is considered
	/// started and the regular healthcheck takes over (`--health-startup-success`).
	#[serde(rename = "Successes", skip_serializing_if = "Option::is_none")]
	pub successes: Option<i64>,
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
