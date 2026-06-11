//! Podman libpod exec API request and response types.

use serde::{Deserialize, Serialize};

/// Request body for `POST /libpod/containers/{name}/exec`.
#[derive(Serialize, Default)]
pub struct ExecCreateConfig {
	#[serde(rename = "Cmd", skip_serializing_if = "Option::is_none")]
	pub cmd: Option<Vec<String>>,

	#[serde(rename = "AttachStdout", skip_serializing_if = "Option::is_none")]
	pub attach_stdout: Option<bool>,

	#[serde(rename = "AttachStderr", skip_serializing_if = "Option::is_none")]
	pub attach_stderr: Option<bool>,

	#[serde(rename = "AttachStdin", skip_serializing_if = "Option::is_none")]
	pub attach_stdin: Option<bool>,

	#[serde(rename = "Tty", skip_serializing_if = "Option::is_none")]
	pub tty: Option<bool>,

	#[serde(rename = "User", skip_serializing_if = "Option::is_none")]
	pub user: Option<String>,

	#[serde(rename = "Privileged", skip_serializing_if = "Option::is_none")]
	pub privileged: Option<bool>,

	#[serde(rename = "WorkingDir", skip_serializing_if = "Option::is_none")]
	pub working_dir: Option<String>,

	#[serde(rename = "Env", skip_serializing_if = "Option::is_none")]
	pub env: Option<Vec<String>>,
}

/// Response from `POST /libpod/containers/{name}/exec`.
#[derive(Deserialize)]
pub struct ExecCreateResponse {
	#[serde(rename = "Id")]
	pub id: String,
}

/// Request body for `POST /libpod/exec/{id}/start`.
#[derive(Serialize)]
pub struct ExecStartConfig {
	#[serde(rename = "Detach")]
	pub detach: bool,

	#[serde(rename = "Tty")]
	pub tty: bool,
}

/// Response from `GET /libpod/exec/{id}/json`.
#[derive(Deserialize, Default)]
pub struct ExecInspect {
	#[serde(rename = "ExitCode")]
	pub exit_code: Option<i64>,
}
