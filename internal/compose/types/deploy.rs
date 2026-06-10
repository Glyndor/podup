//! Deployment configuration types for the `deploy:` service key.
//!
//! These types map to the Docker Swarm / Compose deploy spec and are used by
//! the engine to set resource limits, replica counts, restart policies, and
//! placement constraints. Most fields are optional; absent fields inherit the
//! container runtime defaults.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::Labels;

/// `deploy:` service key — resource limits, replica count, restart policies, and labels.
///
/// The engine uses `replicas`, `resources`, `restart_policy`, and `labels`.
/// Fields inherited from Docker Swarm (`mode`, `placement`, `update_config`,
/// `rollback_config`, `endpoint_mode`) are parsed so existing compose files
/// are accepted without error, but they have no effect on single-host Podman
/// and emit a warning at runtime.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DeployConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub replicas: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub resources: Option<ResourcesConfig>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub restart_policy: Option<DeployRestartPolicy>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub update_config: Option<DeployUpdateConfig>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub rollback_config: Option<DeployUpdateConfig>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub endpoint_mode: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mode: Option<String>,
	#[serde(default)]
	pub labels: Labels,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub placement: Option<DeployPlacement>,
}

/// Resource constraints under `deploy.resources:` — holds `limits` and `reservations`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ResourcesConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub limits: Option<ResourceSpec>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reservations: Option<ResourceSpec>,
}

/// A single resource specification: CPU shares, memory limit, pids limit, and device access.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ResourceSpec {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cpus: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub memory: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub pids: Option<u64>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub devices: Vec<DeviceReservation>,
}

// ---------------------------------------------------------------------------
// Device reservations (GPU / accelerators)
// ---------------------------------------------------------------------------

/// `deploy.resources.reservations.devices` — generic device reservation.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DeviceReservation {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub capabilities: Vec<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub count: Option<CountOrAll>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub device_ids: Vec<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub options: HashMap<String, String>,
}

/// `count: all` or `count: N`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CountOrAll {
	Named(String),
	N(i64),
}

impl CountOrAll {
	pub fn to_i64(&self) -> i64 {
		match self {
			CountOrAll::Named(_) => -1,
			CountOrAll::N(n) => *n,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::CountOrAll;

	#[test]
	fn count_or_all_named_returns_minus_one() {
		assert_eq!(CountOrAll::Named("all".into()).to_i64(), -1);
	}

	#[test]
	fn count_or_all_n_returns_value() {
		assert_eq!(CountOrAll::N(4).to_i64(), 4);
	}
}

/// Restart policy under `deploy.restart_policy:` — distinct from the service-level `restart:` string.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DeployRestartPolicy {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub condition: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub delay: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_attempts: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub window: Option<String>,
}

/// Update (and rollback) configuration — reused for both `deploy.update_config` and `deploy.rollback_config`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DeployUpdateConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parallelism: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub delay: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub failure_action: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub monitor: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_failure_ratio: Option<f64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub order: Option<String>,
}

/// Placement constraints and preferences controlling which nodes a service's containers may run on.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DeployPlacement {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub constraints: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub preferences: Vec<serde_yaml::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_replicas_per_node: Option<u32>,
}
