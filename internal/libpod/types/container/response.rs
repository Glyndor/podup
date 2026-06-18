//! Response types for container API calls.

use std::collections::HashMap;

use serde::Deserialize;

/// Deserialize a collection field, treating JSON `null` as the default.
///
/// Podman sometimes returns `null` instead of `[]`/`{}` for empty collection
/// fields, which would otherwise fail to deserialize into a `Vec`/`HashMap`.
fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
	D: serde::Deserializer<'de>,
	T: Default + Deserialize<'de>,
{
	Option::<T>::deserialize(d).map(|v| v.unwrap_or_default())
}

/// Entry in the `GET /libpod/containers/json` response array.
#[derive(Deserialize)]
pub struct ContainerListEntry {
	#[serde(rename = "Id", default)]
	pub id: String,

	#[serde(rename = "Names", default, deserialize_with = "null_default")]
	pub names: Vec<String>,

	#[serde(rename = "Image", default)]
	pub image: String,

	#[serde(rename = "Status", default)]
	pub status: String,

	/// Machine-readable state (`running`, `exited`, `created`, …). Podman's
	/// libpod list response leaves `Status` empty and reports the state here.
	#[serde(rename = "State", default)]
	pub state: String,

	#[serde(rename = "Ports", default, deserialize_with = "null_default")]
	pub ports: Vec<ContainerPort>,

	#[serde(rename = "Labels", default, deserialize_with = "null_default")]
	pub labels: HashMap<String, String>,
}

/// Port mapping entry in container list response.
#[derive(Deserialize, Default)]
pub struct ContainerPort {
	pub host_ip: Option<String>,
	pub host_port: Option<u16>,
	pub container_port: u16,
}

/// Response from `GET /libpod/containers/{name}/json`.
#[derive(Deserialize, Default)]
pub struct ContainerInspect {
	#[serde(rename = "State")]
	pub state: Option<ContainerState>,

	#[serde(rename = "Config")]
	pub config: Option<ContainerConfig>,

	#[serde(rename = "NetworkSettings")]
	pub network_settings: Option<NetworkSettings>,
}

/// Container config sub-object from inspect.
#[derive(Deserialize, Default)]
pub struct ContainerConfig {
	#[serde(rename = "Healthcheck")]
	pub healthcheck: Option<HealthConfig>,
}

impl ContainerConfig {
	/// Whether the container has an effective healthcheck that can report a
	/// `healthy` status. Covers healthchecks inherited from the image as well
	/// as those declared in compose. A `["NONE"]` test means it was disabled.
	pub fn has_healthcheck(&self) -> bool {
		self.healthcheck
			.as_ref()
			.is_some_and(|h| h.test.first().is_some_and(|first| first != "NONE"))
	}
}

/// Effective healthcheck config baked into the container (image or compose).
#[derive(Deserialize, Default)]
pub struct HealthConfig {
	#[serde(rename = "Test", default, deserialize_with = "null_default")]
	pub test: Vec<String>,
}

/// Container state sub-object.
#[derive(Deserialize, Default)]
pub struct ContainerState {
	// `status`/`exit_code` round-trip the libpod state for completeness and are
	// asserted in tests; container completion now blocks on `wait?condition`
	// (which returns the code directly) rather than reading them here.
	#[allow(dead_code)]
	#[serde(rename = "Status")]
	pub status: Option<String>,

	#[allow(dead_code)]
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

/// Response from `GET /libpod/containers/{name}/top`.
#[derive(Deserialize, Default)]
pub struct TopResponse {
	#[serde(rename = "Titles")]
	pub titles: Option<Vec<String>>,

	#[serde(rename = "Processes")]
	pub processes: Option<Vec<Vec<String>>>,
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::libpod::types::container::spec::Namespace;

	// ---------------------------------------------------------------------------
	// Namespace (spec type — tested here to avoid pushing spec.rs over 500 lines)
	// ---------------------------------------------------------------------------

	#[test]
	fn namespace_new_has_no_value() {
		let ns = Namespace::new("host");
		assert_eq!(ns.nsmode, "host");
		assert!(ns.value.is_none());
	}

	#[test]
	fn namespace_container_sets_value() {
		let ns = Namespace::container("other");
		assert_eq!(ns.nsmode, "container");
		assert_eq!(ns.value.as_deref(), Some("other"));
	}

	#[test]
	fn namespace_parse_container_prefix() {
		let ns = Namespace::parse("container:sidecar");
		assert_eq!(ns.nsmode, "container");
		assert_eq!(ns.value.as_deref(), Some("sidecar"));
	}

	#[test]
	fn namespace_parse_plain_mode() {
		let ns = Namespace::parse("host");
		assert_eq!(ns.nsmode, "host");
		assert!(ns.value.is_none());
	}

	// ---------------------------------------------------------------------------
	// Response deserialization
	// ---------------------------------------------------------------------------

	#[test]
	fn container_inspect_deserialize_healthy() {
		let json = r#"{
			"State": {
				"Status": "running",
				"ExitCode": 0,
				"Health": { "Status": "healthy" }
			}
		}"#;
		let ci: ContainerInspect = serde_json::from_str(json).unwrap();
		let state = ci.state.unwrap();
		assert_eq!(state.status.as_deref(), Some("running"));
		assert_eq!(state.exit_code, Some(0));
		assert_eq!(state.health.unwrap().status.as_deref(), Some("healthy"));
	}

	#[test]
	fn container_inspect_missing_fields_default() {
		let json = r#"{}"#;
		let ci: ContainerInspect = serde_json::from_str(json).unwrap();
		assert!(ci.state.is_none());
		assert!(ci.config.is_none());
		assert!(ci.network_settings.is_none());
	}

	#[test]
	fn has_healthcheck_true_for_image_inherited() {
		let json = r#"{
			"Config": { "Healthcheck": { "Test": ["CMD-SHELL", "curl -f http://localhost || exit 1"] } }
		}"#;
		let ci: ContainerInspect = serde_json::from_str(json).unwrap();
		assert!(ci.config.unwrap().has_healthcheck());
	}

	#[test]
	fn has_healthcheck_false_when_disabled_with_none() {
		let json = r#"{ "Config": { "Healthcheck": { "Test": ["NONE"] } } }"#;
		let ci: ContainerInspect = serde_json::from_str(json).unwrap();
		assert!(!ci.config.unwrap().has_healthcheck());
	}

	#[test]
	fn has_healthcheck_false_when_absent() {
		let json = r#"{ "Config": {} }"#;
		let ci: ContainerInspect = serde_json::from_str(json).unwrap();
		assert!(!ci.config.unwrap().has_healthcheck());
	}

	#[test]
	fn has_healthcheck_false_when_test_null() {
		let json = r#"{ "Config": { "Healthcheck": { "Test": null } } }"#;
		let ci: ContainerInspect = serde_json::from_str(json).unwrap();
		assert!(!ci.config.unwrap().has_healthcheck());
	}

	#[test]
	fn top_response_deserialize() {
		let json = r#"{"Titles": ["PID", "CMD"], "Processes": [["1", "bash"]]}"#;
		let tr: TopResponse = serde_json::from_str(json).unwrap();
		assert_eq!(tr.titles.unwrap(), vec!["PID", "CMD"]);
		assert_eq!(tr.processes.unwrap(), vec![vec!["1", "bash"]]);
	}

	#[test]
	fn container_list_entry_default_fields() {
		let json =
			r#"{"Names": ["/mycontainer"], "Image": "nginx", "Status": "running", "Ports": []}"#;
		let entry: ContainerListEntry = serde_json::from_str(json).unwrap();
		assert_eq!(entry.names, vec!["/mycontainer"]);
		assert_eq!(entry.image, "nginx");
		assert_eq!(entry.status, "running");
	}

	#[test]
	fn container_list_entry_null_vec_fields() {
		let json = r#"{"Names": null, "Image": "alpine", "Status": "exited", "Ports": null}"#;
		let entry: ContainerListEntry = serde_json::from_str(json).unwrap();
		assert!(entry.names.is_empty());
		assert!(entry.ports.is_empty());
	}
}
