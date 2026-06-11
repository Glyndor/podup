//! Podman libpod volume API request and response types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Request body for `POST /libpod/volumes/create`.
#[derive(Serialize, Default)]
pub struct VolumeCreateOptions {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,

	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub driver_opts: HashMap<String, String>,

	#[serde(skip_serializing_if = "HashMap::is_empty", default)]
	pub labels: HashMap<String, String>,
}

/// Response from volume creation or inspection.
#[allow(dead_code)]
#[derive(Deserialize)]
pub struct VolumeResponse {
	#[serde(rename = "Name")]
	pub name: String,
}
