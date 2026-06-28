//! `exec` — run a command in a running service container (`docker compose
//! exec`), split out of `query/mod.rs` to keep that file within the source line
//! limit as the CLI surface grows.

use std::io::Write;

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::exec::{
	ExecCreateConfig, ExecCreateResponse, ExecInspect, ExecStartConfig,
};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

use super::Engine;

/// Options for [`Engine::exec`], mirroring `docker compose exec` flags.
#[derive(Default)]
pub struct ExecOptions {
	/// Extra environment variables (`KEY=VAL`), `-e/--env`.
	pub env: Vec<String>,
	/// Run as this user, `-u/--user`.
	pub user: Option<String>,
	/// Working directory inside the container, `-w/--workdir`.
	pub workdir: Option<String>,
	/// Run with extended privileges, `--privileged`.
	pub privileged: bool,
	/// Detach: start the exec and return without streaming output, `-d/--detach`.
	pub detach: bool,
	/// 1-based replica index for a scaled service, `--index` (default: first).
	pub index: Option<u32>,
}

impl Engine {
	/// Run a command in the first replica of the named service with default
	/// options. Exits with the command's exit code.
	pub async fn exec(
		&self,
		file: &ComposeFile,
		service_name: &str,
		cmd: Vec<String>,
	) -> Result<()> {
		self.exec_with_options(file, service_name, cmd, ExecOptions::default())
			.await
	}

	/// Run a command in a service container with `docker compose exec`-style
	/// overrides (env, user, workdir, privileged, detach, replica index).
	pub async fn exec_with_options(
		&self,
		file: &ComposeFile,
		service_name: &str,
		cmd: Vec<String>,
		opts: ExecOptions,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = match opts.index {
			Some(i) => {
				let names = self.replica_names(service_name, service);
				let idx = (i as usize).saturating_sub(1);
				names.get(idx).cloned().ok_or_else(|| {
					ComposeError::ServiceNotFound(format!("{service_name} (replica index {i})"))
				})?
			}
			None => self.first_replica_name(service_name, service),
		};

		let exec_cfg = ExecCreateConfig {
			cmd: Some(cmd),
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			user: opts.user.clone(),
			working_dir: opts.workdir.clone(),
			privileged: opts.privileged.then_some(true),
			env: (!opts.env.is_empty()).then(|| opts.env.clone()),
			..Default::default()
		};
		let create_path = format!(
			"{API_PREFIX}/containers/{}/exec",
			urlencoded(&container_name),
		);
		let resp: ExecCreateResponse = self
			.client
			.post_json(&create_path, &exec_cfg)
			.await
			.map_err(ComposeError::Podman)?;
		let exec_id = resp.id;

		// `-d/--detach`: start the exec and return without streaming output or
		// waiting for the exit code. The server returns immediately, so the
		// response body is dropped.
		if opts.detach {
			let start_cfg = ExecStartConfig {
				detach: true,
				tty: false,
			};
			let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(&exec_id));
			let _ = self
				.client
				.post_json_stream(&start_path, &start_cfg)
				.await
				.map_err(ComposeError::Podman)?;
			return Ok(());
		}

		let start_cfg = ExecStartConfig {
			detach: false,
			tty: false,
		};
		let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(&exec_id));
		let start_resp = self
			.client
			.post_json_stream(&start_path, &start_cfg)
			.await
			.map_err(ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_multiplexed(start_resp.into_body());

		// Lock stdout once for the whole stream instead of re-acquiring the lock
		// (and issuing a syscall) per frame; stdout is ours exclusively on this
		// path. stderr is locked per frame because the tracing subscriber also
		// writes there: holding its lock across the await loop would starve
		// concurrent log emissions. Flush after each frame so exec streams
		// promptly.
		{
			let mut out = std::io::stdout().lock();
			while let Some(msg) = stream.next().await {
				match msg.map_err(ComposeError::Podman)? {
					LogOutput::StdOut { message } => {
						let _ = out.write_all(String::from_utf8_lossy(&message).as_bytes());
						let _ = out.flush();
					}
					LogOutput::StdErr { message } => {
						let mut err = std::io::stderr().lock();
						let _ = err.write_all(String::from_utf8_lossy(&message).as_bytes());
						let _ = err.flush();
					}
				}
			}
		}

		let inspect_path = format!("{API_PREFIX}/exec/{}/json", urlencoded(&exec_id));
		let inspect: ExecInspect = self
			.client
			.get_json(&inspect_path)
			.await
			.map_err(ComposeError::Podman)?;
		if let Some(code) = inspect.exit_code {
			if code != 0 {
				return Err(ComposeError::RunExited(code));
			}
		}

		Ok(())
	}
}
