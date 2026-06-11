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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn exec_create_config_serializes_cmd() {
		let cfg = ExecCreateConfig {
			cmd: Some(vec!["sh".into(), "-c".into(), "echo hi".into()]),
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			..Default::default()
		};
		let v = serde_json::to_value(&cfg).unwrap();
		assert_eq!(v["Cmd"], serde_json::json!(["sh", "-c", "echo hi"]));
		assert_eq!(v["AttachStdout"], serde_json::json!(true));
	}

	#[test]
	fn exec_create_config_skips_none_fields() {
		let cfg = ExecCreateConfig::default();
		let v = serde_json::to_value(&cfg).unwrap();
		assert!(v.get("Cmd").is_none());
		assert!(v.get("User").is_none());
	}

	#[test]
	fn exec_start_config_serializes() {
		let cfg = ExecStartConfig { detach: false, tty: true };
		let v = serde_json::to_value(&cfg).unwrap();
		assert_eq!(v["Detach"], serde_json::json!(false));
		assert_eq!(v["Tty"], serde_json::json!(true));
	}

	#[test]
	fn exec_create_response_deserialize() {
		let json = r#"{"Id": "abc123"}"#;
		let r: ExecCreateResponse = serde_json::from_str(json).unwrap();
		assert_eq!(r.id, "abc123");
	}

	#[test]
	fn exec_inspect_deserialize() {
		let json = r#"{"ExitCode": 1}"#;
		let r: ExecInspect = serde_json::from_str(json).unwrap();
		assert_eq!(r.exit_code, Some(1));
	}
}
