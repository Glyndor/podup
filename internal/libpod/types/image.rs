//! Podman libpod image API request and response types.

use serde::Deserialize;

/// Streaming JSON line emitted during image pull (`POST /libpod/images/pull`).
#[derive(Deserialize, Default)]
pub struct ImagePullProgress {
	#[serde(default)]
	pub stream: String,

	#[serde(default)]
	pub error: String,
}

/// Streaming JSON line emitted during image build (`POST /libpod/build`).
#[derive(Deserialize, Default)]
pub struct BuildOutput {
	#[serde(default)]
	pub stream: String,

	pub error: Option<String>,

	pub error_detail: Option<BuildErrorDetail>,
}

/// Error detail sub-object in build output.
#[derive(Deserialize)]
pub struct BuildErrorDetail {
	pub message: Option<String>,
}

/// Response from `GET /libpod/images/{name}/json`.
#[derive(Deserialize, Default)]
pub struct ImageInspect {
	#[serde(rename = "Id", default)]
	pub id: String,
}
