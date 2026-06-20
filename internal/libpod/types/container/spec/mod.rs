//! SpecGenerator and all nested request types for container creation.

use std::collections::HashMap;

use serde::Serialize;

mod parts;
pub use parts::*;

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

	/// Stop signal as a numeric `syscall.Signal`. libpod rejects a string here
	/// with HTTP 500, so the compose `stop_signal:` name is resolved to its
	/// integer number before being sent.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub stop_signal: Option<i64>,

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

	// Podman's SpecGenerator names this field `hostadd` (there is no `extra_hosts`
	// key); without the rename every extra_hosts entry is silently dropped.
	#[serde(rename = "hostadd", skip_serializing_if = "Vec::is_empty", default)]
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
	pub volumes: Vec<NamedVolume>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub volumes_from: Vec<String>,

	// Podman-native secrets (also used for external configs): each references an
	// existing `podman secret` by name and is mounted into the container.
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub secrets: Vec<Secret>,

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

	// Podman's SpecGenerator names this field `r_limits` (POSIX rlimits); without
	// the rename every ulimits entry is silently dropped. The per-element shape
	// (`{type, soft, hard}`) already matches Podman's POSIXRlimit.
	#[serde(rename = "r_limits", skip_serializing_if = "Vec::is_empty", default)]
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

	/// CDI device names (e.g. `nvidia.com/gpu=all`) for GPU/accelerator access.
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub cdi_devices: Vec<String>,

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

#[cfg(test)]
mod tests {
	use super::{SpecGenerator, Ulimit};

	#[test]
	fn extra_hosts_serialize_as_hostadd() {
		let spec = SpecGenerator {
			extra_hosts: vec!["db:10.0.0.2".to_string()],
			..Default::default()
		};
		let v = serde_json::to_value(&spec).unwrap();
		// Podman's SpecGenerator key is `hostadd`; `extra_hosts` matches no field
		// and is silently dropped.
		assert_eq!(v["hostadd"][0], "db:10.0.0.2");
		assert!(v.get("extra_hosts").is_none(), "stale extra_hosts key: {v}");
	}

	#[test]
	fn ulimits_serialize_as_r_limits_with_posix_shape() {
		let spec = SpecGenerator {
			ulimits: vec![Ulimit {
				ulimit_type: "nofile".to_string(),
				soft: 1024,
				hard: 2048,
			}],
			..Default::default()
		};
		let v = serde_json::to_value(&spec).unwrap();
		// Podman's key is `r_limits`; the element shape is POSIXRlimit {type, soft, hard}.
		assert!(v.get("ulimits").is_none(), "stale ulimits key: {v}");
		assert_eq!(v["r_limits"][0]["type"], "nofile");
		assert_eq!(v["r_limits"][0]["soft"], 1024);
		assert_eq!(v["r_limits"][0]["hard"], 2048);
	}
}
