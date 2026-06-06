//! Volume, secret, and config mount types.
//!
//! [`VolumeMount`] covers the `volumes:` list on a service (short and long forms).
//! [`VolumeConfig`] describes top-level named volume definitions.
//! [`ServiceSecretRef`] and [`ServiceConfigRef`] are the per-service `secrets:` /
//! `configs:` attachment points (short = just the name, long = full options).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::Labels;

// ---------------------------------------------------------------------------
// VolumeType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VolumeType {
	Volume,
	Bind,
	Tmpfs,
	Npipe,
	Cluster,
}

// ---------------------------------------------------------------------------
// Mount options
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BindOptions {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub propagation: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub create_host_path: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub selinux: Option<String>,
}

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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DriverConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub options: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TmpfsOptions {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub size: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mode: Option<u32>,
}

// ---------------------------------------------------------------------------
// VolumeMount
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Top-level VolumeConfig
// ---------------------------------------------------------------------------

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
}

// ---------------------------------------------------------------------------
// Service-level config / secret references
// ---------------------------------------------------------------------------

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
