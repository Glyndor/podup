//! Response types for container API calls.

use std::collections::HashMap;

use serde::Deserialize;

/// Entry in the `GET /libpod/containers/json` response array.
#[derive(Deserialize)]
pub struct ContainerListEntry {
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
