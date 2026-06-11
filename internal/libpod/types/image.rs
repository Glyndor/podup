//! Podman libpod image API request and response types.

use serde::{Deserialize, Serialize};

/// Streaming JSON line emitted during image pull (`POST /libpod/images/pull`).
#[derive(Deserialize, Default)]
pub struct ImagePullProgress {
	#[serde(default)]
	pub stream: String,

	#[serde(default)]
	pub id: String,

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

	#[serde(rename = "RepoTags", default)]
	pub repo_tags: Vec<String>,
}

/// Query parameters for image build (sent as URL query string).
#[derive(Serialize, Default)]
pub struct BuildQueryParams {
	pub t: String,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub dockerfile: Option<String>,

	pub rm: bool,

	pub nocache: bool,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub pull: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub platform: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub networkmode: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub shmsize: Option<i32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub extrahosts: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub cachefrom: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub buildargs: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub labels: Option<String>,
}
