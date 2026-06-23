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

	// Podman's SpecGenerator has no `security_opt` field; the compose list is
	// decomposed into these dedicated fields. A plain `security_opt` array is
	// silently ignored, so every option (incl. no-new-privileges/seccomp/apparmor)
	// would otherwise be dropped.
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub selinux_opts: Vec<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub apparmor_profile: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub seccomp_profile_path: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub no_new_privileges: Option<bool>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub mask: Vec<String>,

	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub unmask: Vec<String>,

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

	/// What Podman does when the healthcheck flips to unhealthy (Podman 5's
	/// `--health-on-failure`). Podman's key is `health_check_on_failure_action`.
	#[serde(
		rename = "health_check_on_failure_action",
		skip_serializing_if = "Option::is_none"
	)]
	pub health_check_on_failure_action: Option<HealthCheckOnFailureAction>,

	/// Separate startup-phase healthcheck (Podman 5's `--health-startup-*`).
	/// Podman's key is `startupHealthConfig`.
	#[serde(
		rename = "startupHealthConfig",
		skip_serializing_if = "Option::is_none"
	)]
	pub startup_health_config: Option<StartupHealthCheck>,

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

	// Devices. Podman 5.x has no SpecGenerator CDI field; CDI device names (e.g.
	// `nvidia.com/gpu=all`) are recognized by ExtractCDIDevices from this array by
	// their qualified path, so they are appended here as `LinuxDevice` entries.
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub devices: Vec<LinuxDevice>,

	// Podman expects structured cgroup rules ([]LinuxDeviceCgroup), not strings; a
	// string array would fail to deserialize.
	#[serde(skip_serializing_if = "Vec::is_empty", default)]
	pub device_cgroup_rule: Vec<LinuxDeviceCgroup>,

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
	use super::{
		HealthCheckOnFailureAction, HealthConfig, LinuxDeviceCgroup, SpecGenerator,
		StartupHealthCheck, Ulimit,
	};

	#[test]
	fn security_fields_serialize_decomposed_not_as_security_opt() {
		let spec = SpecGenerator {
			selinux_opts: vec!["disable".to_string()],
			apparmor_profile: Some("prof".to_string()),
			seccomp_profile_path: Some("unconfined".to_string()),
			no_new_privileges: Some(true),
			mask: vec!["/proc/kcore".to_string()],
			unmask: vec!["ALL".to_string()],
			..Default::default()
		};
		let v = serde_json::to_value(&spec).unwrap();
		// SpecGenerator has no `security_opt` field — the value must arrive decomposed.
		assert!(
			v.get("security_opt").is_none(),
			"stale security_opt key: {v}"
		);
		assert_eq!(v["selinux_opts"][0], "disable");
		assert_eq!(v["apparmor_profile"], "prof");
		assert_eq!(v["seccomp_profile_path"], "unconfined");
		assert_eq!(v["no_new_privileges"], true);
		assert_eq!(v["mask"][0], "/proc/kcore");
		assert_eq!(v["unmask"][0], "ALL");
	}

	#[test]
	fn device_cgroup_rule_serializes_as_struct_array() {
		let spec = SpecGenerator {
			device_cgroup_rule: vec![LinuxDeviceCgroup {
				allow: true,
				device_type: Some("c".to_string()),
				major: Some(1),
				minor: None,
				access: Some("rwm".to_string()),
			}],
			..Default::default()
		};
		let v = serde_json::to_value(&spec).unwrap();
		// Podman expects []LinuxDeviceCgroup objects, not strings.
		assert_eq!(v["device_cgroup_rule"][0]["allow"], true);
		assert_eq!(v["device_cgroup_rule"][0]["type"], "c");
		assert_eq!(v["device_cgroup_rule"][0]["major"], 1);
		// minor=None must be omitted (means "all").
		assert!(v["device_cgroup_rule"][0].get("minor").is_none());
		assert_eq!(v["device_cgroup_rule"][0]["access"], "rwm");
	}

	#[test]
	fn no_cdi_devices_key_is_emitted() {
		// Podman 5.x has no cdi_devices field; CDI names ride in `devices`.
		let v = serde_json::to_value(SpecGenerator::default()).unwrap();
		assert!(v.get("cdi_devices").is_none(), "stale cdi_devices key: {v}");
	}

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

	#[test]
	fn health_on_failure_and_startup_use_podman_wire_names() {
		let spec = SpecGenerator {
			health_check_on_failure_action: Some(HealthCheckOnFailureAction::Restart),
			startup_health_config: Some(StartupHealthCheck {
				health_config: HealthConfig {
					test: Some(vec!["CMD".to_string(), "true".to_string()]),
					interval: Some(1_000_000_000),
					..Default::default()
				},
				successes: Some(3),
			}),
			..Default::default()
		};
		let v = serde_json::to_value(&spec).unwrap();

		// `--health-on-failure` rides as Podman's integer action code (restart = 3),
		// under the snake_case key — not as a string and not as `none`(0).
		assert_eq!(v["health_check_on_failure_action"], 3);

		// The startup probe nests under the PascalCase `startupHealthConfig` key,
		// with its embedded probe fields flattened (PascalCase) and `Successes`.
		let startup = &v["startupHealthConfig"];
		assert_eq!(startup["Test"][0], "CMD");
		assert_eq!(startup["Test"][1], "true");
		assert_eq!(startup["Interval"], 1_000_000_000_i64);
		assert_eq!(startup["Successes"], 3);
		// Flattened — there is no nested `health_config` wrapper key.
		assert!(startup.get("health_config").is_none(), "not flattened: {v}");
	}

	#[test]
	fn health_fields_omitted_when_unset() {
		// Both new fields are `Option` and must vanish from the wire when unset.
		let v = serde_json::to_value(SpecGenerator::default()).unwrap();
		assert!(v.get("health_check_on_failure_action").is_none());
		assert!(v.get("startupHealthConfig").is_none());
	}
}
