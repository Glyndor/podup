//! Podman libpod volume API request and response types.

use std::collections::HashMap;

use serde::Serialize;

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
