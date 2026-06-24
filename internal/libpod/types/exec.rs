//! Podman libpod exec API request and response types.

use serde::{Deserialize, Serialize};

/// Request body for `POST /libpod/containers/{name}/exec`.
#[derive(Serialize, Default)]
pub struct ExecCreateConfig {
	/// Command and arguments to run, as an argv vector (not shell-parsed).
	#[serde(rename = "Cmd", skip_serializing_if = "Option::is_none")]
	pub cmd: Option<Vec<String>>,

	/// Whether to attach to the exec process's stdout.
	#[serde(rename = "AttachStdout", skip_serializing_if = "Option::is_none")]
	pub attach_stdout: Option<bool>,

	/// Whether to attach to the exec process's stderr.
	#[serde(rename = "AttachStderr", skip_serializing_if = "Option::is_none")]
	pub attach_stderr: Option<bool>,

	/// Whether to attach to the exec process's stdin.
	#[serde(rename = "AttachStdin", skip_serializing_if = "Option::is_none")]
	pub attach_stdin: Option<bool>,

	/// Whether to allocate a pseudo-TTY for the exec process.
	#[serde(rename = "Tty", skip_serializing_if = "Option::is_none")]
	pub tty: Option<bool>,

	/// User (and optionally group) to run the exec process as (`user[:group]`).
	#[serde(rename = "User", skip_serializing_if = "Option::is_none")]
	pub user: Option<String>,

	/// Whether to run the exec process with extended (privileged) permissions.
	#[serde(rename = "Privileged", skip_serializing_if = "Option::is_none")]
	pub privileged: Option<bool>,

	/// Working directory inside the container for the exec process.
	#[serde(rename = "WorkingDir", skip_serializing_if = "Option::is_none")]
	pub working_dir: Option<String>,

	/// Environment variables for the exec process, each entry `KEY=VALUE`.
	#[serde(rename = "Env", skip_serializing_if = "Option::is_none")]
	pub env: Option<Vec<String>>,
}

/// Response from `POST /libpod/containers/{name}/exec`.
#[derive(Deserialize)]
pub struct ExecCreateResponse {
	/// Exec session ID, used to start and inspect the session.
	#[serde(rename = "Id")]
	pub id: String,
}

/// Request body for `POST /libpod/exec/{id}/start`.
#[derive(Serialize)]
pub struct ExecStartConfig {
	/// Whether to start the exec session detached rather than streaming output.
	#[serde(rename = "Detach")]
	pub detach: bool,

	/// Whether the exec session was created with a TTY; selects raw vs.
	/// multiplexed stream framing on the start response.
	#[serde(rename = "Tty")]
	pub tty: bool,
}

/// Response from `GET /libpod/exec/{id}/json`.
#[derive(Deserialize, Default)]
pub struct ExecInspect {
	/// Exit code of the finished exec process; `None` while it is still running.
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
		let cfg = ExecStartConfig {
			detach: false,
			tty: true,
		};
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
