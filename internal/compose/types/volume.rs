//! Volume, secret, and config mount types.
//!
//! [`VolumeMount`] covers the `volumes:` list on a service (short and long forms).
//! [`VolumeConfig`] describes top-level named volume definitions.
//! [`ServiceSecretRef`] and [`ServiceConfigRef`] are the per-service `secrets:` /
//! `configs:` attachment points (short = just the name, long = full options).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::Labels;

/// Volume mount type: `volume`, `bind`, `tmpfs`, `npipe`, or `cluster`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VolumeType {
	Volume,
	Bind,
	Tmpfs,
	Npipe,
	Cluster,
}

/// Sub-options for a `bind`-type volume mount.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BindOptions {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub propagation: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub create_host_path: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub selinux: Option<String>,
}

/// Sub-options for a `volume`-type mount — nocopy flag and optional driver config.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct VolumeOptions {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub nocopy: Option<bool>,
	#[serde(default)]
	pub labels: Labels,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver_config: Option<DriverConfig>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub subpath: Option<String>,
}

/// Driver name and key-value options nested under `VolumeOptions`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DriverConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub options: HashMap<String, String>,
}

/// Sub-options for a `tmpfs`-type mount — size and mode.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TmpfsOptions {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub size: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mode: Option<u32>,
}

/// A volume mount entry — either a short-form string or a long-form typed block.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum VolumeMount {
	Short(String),
	Long {
		#[serde(rename = "type")]
		volume_type: VolumeType,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		source: Option<String>,
		target: String,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		read_only: Option<bool>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		bind: Option<BindOptions>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		volume: Option<VolumeOptions>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		tmpfs: Option<TmpfsOptions>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		consistency: Option<String>,
	},
}

impl VolumeMount {
	pub fn target(&self) -> &str {
		match self {
			VolumeMount::Short(s) => {
				let parts: Vec<&str> = s.splitn(3, ':').collect();
				if parts.len() >= 2 {
					parts[1]
				} else {
					parts[0]
				}
			}
			VolumeMount::Long { target, .. } => target,
		}
	}
}

/// Named volume definition in the top-level `volumes:` block.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct VolumeConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub driver_opts: HashMap<String, String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub external: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(default)]
	pub labels: Labels,
	#[serde(flatten, default, skip_serializing_if = "indexmap::IndexMap::is_empty")]
	pub unknown: indexmap::IndexMap<String, serde_yaml::Value>,
}

/// Reference to a named config from a service — short form (name only) or long form with mount target.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ServiceConfigRef {
	Short(String),
	Long {
		source: String,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		target: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		uid: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		gid: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		mode: Option<u32>,
	},
}

impl ServiceConfigRef {
	pub fn source(&self) -> &str {
		match self {
			ServiceConfigRef::Short(s) => s,
			ServiceConfigRef::Long { source, .. } => source,
		}
	}

	pub fn target(&self) -> Option<&str> {
		match self {
			ServiceConfigRef::Short(_) => None,
			ServiceConfigRef::Long { target, .. } => target.as_deref(),
		}
	}
}

/// Reference to a named secret from a service — short form (name only) or long form with mount target.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ServiceSecretRef {
	Short(String),
	Long {
		source: String,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		target: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		uid: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		gid: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		mode: Option<u32>,
	},
}

impl ServiceSecretRef {
	pub fn source(&self) -> &str {
		match self {
			ServiceSecretRef::Short(s) => s,
			ServiceSecretRef::Long { source, .. } => source,
		}
	}

	pub fn target(&self) -> Option<&str> {
		match self {
			ServiceSecretRef::Short(_) => None,
			ServiceSecretRef::Long { target, .. } => target.as_deref(),
		}
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	// VolumeMount::target

	#[test]
	fn volume_mount_short_two_parts_returns_second() {
		let m = VolumeMount::Short("./data:/app/data".to_string());
		assert_eq!(m.target(), "/app/data");
	}

	#[test]
	fn volume_mount_short_three_parts_returns_second() {
		let m = VolumeMount::Short("./data:/app/data:ro".to_string());
		assert_eq!(m.target(), "/app/data");
	}

	#[test]
	fn volume_mount_short_no_colon_returns_whole_string() {
		let m = VolumeMount::Short("/app/data".to_string());
		assert_eq!(m.target(), "/app/data");
	}

	#[test]
	fn volume_mount_long_returns_target_field() {
		let m = VolumeMount::Long {
			volume_type: VolumeType::Bind,
			source: Some("/host/path".to_string()),
			target: "/container/path".to_string(),
			read_only: None,
			bind: None,
			volume: None,
			tmpfs: None,
			consistency: None,
		};
		assert_eq!(m.target(), "/container/path");
	}

	// ServiceConfigRef

	#[test]
	fn config_ref_short_source() {
		let r = ServiceConfigRef::Short("my-config".to_string());
		assert_eq!(r.source(), "my-config");
		assert!(r.target().is_none());
	}

	#[test]
	fn config_ref_long_source_and_target() {
		let r = ServiceConfigRef::Long {
			source: "my-config".to_string(),
			target: Some("/run/configs/my-config".to_string()),
			uid: None,
			gid: None,
			mode: None,
		};
		assert_eq!(r.source(), "my-config");
		assert_eq!(r.target(), Some("/run/configs/my-config"));
	}

	#[test]
	fn config_ref_long_no_target() {
		let r = ServiceConfigRef::Long {
			source: "my-config".to_string(),
			target: None,
			uid: None,
			gid: None,
			mode: None,
		};
		assert!(r.target().is_none());
	}

	// ServiceSecretRef

	#[test]
	fn secret_ref_short_source() {
		let r = ServiceSecretRef::Short("my-secret".to_string());
		assert_eq!(r.source(), "my-secret");
		assert!(r.target().is_none());
	}

	#[test]
	fn secret_ref_long_source_and_target() {
		let r = ServiceSecretRef::Long {
			source: "my-secret".to_string(),
			target: Some("/run/secrets/my-secret".to_string()),
			uid: None,
			gid: None,
			mode: None,
		};
		assert_eq!(r.source(), "my-secret");
		assert_eq!(r.target(), Some("/run/secrets/my-secret"));
	}
}
