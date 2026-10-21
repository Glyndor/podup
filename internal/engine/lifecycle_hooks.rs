//! Lifecycle hooks: run a hook's command inside a container via the libpod
//! `/exec` round trip and surface its exit code as a result.
//!
//! Lifted out of `engine::mod` so the engine module stays under the 500-line
//! hard cap enforced by the org's `line-limit` reusable, and so the
//! `pre_start`/`post_start`/`pre_stop` plumbing stays in one file rather
//! than a single 90-line method on `Engine`. The method is still on `Engine`
//! so call sites do not change.

use std::io::Write;

use futures_util::StreamExt;

use crate::compose::types::LifecycleHook;
use crate::engine::write_frame;
use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::types::exec::{ExecCreateConfig, ExecStartConfig};
use crate::libpod::{LogOutput, API_PREFIX};

impl Engine {
	pub(super) async fn run_lifecycle_hook(
		&self,
		container_name: &str,
		hook: &LifecycleHook,
	) -> Result<()> {
		let cmd = hook.command.to_exec();
		let env: Vec<String> = {
			let m = hook.environment.to_map();
			m.into_iter()
				.filter_map(|(k, v)| v.map(|v| format!("{k}={v}")))
				.collect()
		};

		let exec_cfg = ExecCreateConfig {
			cmd: Some(cmd),
			user: hook.user.clone(),
			privileged: hook.privileged,
			working_dir: hook.working_dir.clone(),
			env: if env.is_empty() { None } else { Some(env) },
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			..Default::default()
		};

		let path = format!(
			"{API_PREFIX}/containers/{}/exec",
			crate::libpod::urlencoded(container_name)
		);
		let resp: crate::libpod::types::exec::ExecCreateResponse = self
			.client
			.post_json(&path, &exec_cfg)
			.await
			.map_err(ComposeError::Podman)?;
		let exec_id = resp.id;

		let start_cfg = ExecStartConfig {
			detach: false,
			tty: false,
		};
		let start_path = format!(
			"{API_PREFIX}/exec/{}/start",
			crate::libpod::urlencoded(&exec_id)
		);
		let resp = self
			.client
			.post_json_stream(&start_path, &start_cfg)
			.await
			.map_err(ComposeError::Podman)?;

		let mut stream = crate::libpod::parse_multiplexed(resp.into_body());
		// Lock both stdout and stderr per frame, symmetric with the rest of
		// the engine: holding either lock across the await loop would stall any
		// concurrent log emission, including the tracing subscriber that
		// writes to stderr. The previous code held stdout's lock for the
		// whole stream, which serialised the hook behind any other writer for
		// its entire lifetime (#1369). Flush after each frame so output
		// stays prompt.
		while let Some(msg) = stream.next().await {
			match msg.map_err(ComposeError::Podman)? {
				LogOutput::StdOut { message } => {
					let mut out = std::io::stdout().lock();
					let _ = write_frame(&mut out, &message);
					let _ = out.flush();
				}
				LogOutput::StdErr { message } => {
					let mut err = std::io::stderr().lock();
					let _ = write_frame(&mut err, &message);
					let _ = err.flush();
				}
			}
		}

		// A hook that exits non-zero must surface as an error (matching
		// `Engine::run`): otherwise a failing `post_start` readiness/init step is
		// silently treated as success and dependents start against a container
		// that never initialised. `pre_stop` callers deliberately ignore the Err.
		let inspect_path = format!(
			"{API_PREFIX}/exec/{}/json",
			crate::libpod::urlencoded(&exec_id)
		);
		let inspect: crate::libpod::types::exec::ExecInspect = self
			.client
			.get_json(&inspect_path)
			.await
			.map_err(ComposeError::Podman)?;
		if let Some(code) = inspect.exit_code {
			if code != 0 {
				return Err(ComposeError::Build(format!(
					"lifecycle hook exited with status {code}"
				)));
			}
		}

		Ok(())
	}
}
