//! `exec` command: run a command inside a service container.

use std::io::Write;

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::exec::{
	ExecCreateConfig, ExecCreateResponse, ExecInspect, ExecStartConfig,
};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

use super::Engine;

/// Ceiling on how long to wait for the libpod exec-start *response head*. A
/// healthy engine returns it almost instantly — either the hijacked stream or a
/// prompt error (e.g. HTTP 500 "unable to find user … no matching entries in
/// passwd file"). When the target user/workdir does not resolve, some engine
/// builds instead stall before answering, which the default client read timeout
/// would only abort after ~120s and then report as a misleading socket-timeout.
/// Bounding the head here lets [`Engine::exec_with_options`] fail fast with a
/// clear, exec-specific message. It covers only the head; the streamed exec
/// output is left unbounded so a legitimate long-running command runs to
/// completion.
const EXEC_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

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

/// Build the exec environment list, expanding a bare `KEY` (no `=`) from podup's
/// own environment the way docker-compose does. A `KEY=VALUE` entry passes
/// through unchanged; a value-less `KEY` is replaced with `KEY=<host value>` and
/// dropped entirely when the variable is unset (libpod rejects a bare key with
/// HTTP 400). Pure so it is unit-tested without a container.
fn expand_exec_env(env: &[String]) -> Vec<String> {
	env.iter()
		.filter_map(|e| {
			if e.contains('=') {
				Some(e.clone())
			} else {
				std::env::var(e).ok().map(|v| format!("{e}={v}"))
			}
		})
		.collect()
}

/// True for the in-band stream-teardown line Podman/conmon emits on the exec
/// stderr channel when an exec launch fails (e.g. a bad `--workdir`/`--user`):
/// a secondary `read unixpacket ... connection reset by peer` frame that adds
/// nothing to the real diagnostic. Matching is deliberately narrow so ordinary
/// program output is never suppressed.
fn is_exec_teardown_noise(line: &str) -> bool {
	line.contains("unixpacket") && line.contains("connection reset by peer")
}

/// Map a libpod error from an `exec` target into a friendly
/// [`ComposeError::NotRunning`] when it means the container is absent (404) or
/// stopped ("can only create exec sessions on running containers"), so the user
/// sees "service X is not running" instead of a raw HTTP 404/500. Any other
/// failure passes through unchanged. Pure so it is unit-tested.
fn map_not_running(e: crate::libpod::PodmanError, service_name: &str) -> ComposeError {
	let not_running = e.is_status(404)
		|| matches!(
			&e,
			crate::libpod::PodmanError::Api { message, .. }
				if {
					let m = message.to_ascii_lowercase();
					m.contains("can only create exec sessions on running containers")
						|| m.contains("is not running")
						|| m.contains("no such container")
				}
		);
	if not_running {
		ComposeError::NotRunning(service_name.to_string())
	} else {
		ComposeError::Podman(e)
	}
}

/// Translate a failure *starting* the exec session into a clear error. A
/// client-side timeout means the libpod exec-start never returned its response
/// head within [`EXEC_START_TIMEOUT`] — almost always a wedged launch (e.g. a
/// nonexistent `--user`/`--workdir` the server stalls resolving) rather than an
/// unhealthy socket, so surface that with the likely cause instead of the
/// generic "timed out waiting for the Podman socket" message. Every other error
/// — including the prompt HTTP error an engine *does* return for a bad user
/// ("unable to find user … no matching entries in passwd file") — passes through
/// unchanged so legitimate diagnostics are never masked. Pure so it is
/// unit-tested.
fn map_exec_start_err(e: crate::libpod::PodmanError, opts: &ExecOptions) -> ComposeError {
	if e.is_timeout() {
		let cause = if let Some(user) = &opts.user {
			format!(" (the requested user '{user}' may not exist in the container)")
		} else if let Some(dir) = &opts.workdir {
			format!(" (the requested working directory '{dir}' may not exist)")
		} else {
			String::new()
		};
		return ComposeError::ExecFailed(format!(
			"the exec session did not start within {}s{cause}",
			EXEC_START_TIMEOUT.as_secs()
		));
	}
	ComposeError::Podman(e)
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
		// An empty command would be forwarded as an empty `cmd` and surface a raw
		// podman HTTP 500; reject it up front with a clear message.
		if cmd.is_empty() {
			return Err(ComposeError::Unsupported(
				"exec: a command is required".into(),
			));
		}
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		// Resolve the target replica against the *running* containers (matching
		// `cp`), so `--index N` and a bare `exec` reach a live replica of a service
		// scaled by an earlier `up --scale`/`scale` — not just the compose static
		// count. `--index 0`/out-of-range indexes stay rejected consistently.
		let container_name = self
			.live_replica_name_at(service_name, service, opts.index)
			.await?;

		let env = expand_exec_env(&opts.env);
		let exec_cfg = ExecCreateConfig {
			cmd: Some(cmd),
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			user: opts.user.clone(),
			working_dir: opts.workdir.clone(),
			privileged: opts.privileged.then_some(true),
			env: (!env.is_empty()).then_some(env),
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
			.map_err(|e| map_not_running(e, service_name))?;
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
				.post_json_stream_within(&start_path, &start_cfg, Some(EXEC_START_TIMEOUT))
				.await
				.map_err(|e| map_exec_start_err(e, &opts))?;
			return Ok(());
		}

		let start_cfg = ExecStartConfig {
			detach: false,
			tty: false,
		};
		let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(&exec_id));
		let start_resp = self
			.client
			.post_json_stream_within(&start_path, &start_cfg, Some(EXEC_START_TIMEOUT))
			.await
			.map_err(|e| map_exec_start_err(e, &opts))?;
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
						let text = String::from_utf8_lossy(&message);
						// Drop the spurious connection-reset teardown frame an OCI
						// exec launch-failure emits, so only the real diagnostic shows.
						if is_exec_teardown_noise(&text) {
							continue;
						}
						let mut err = std::io::stderr().lock();
						let _ = err.write_all(text.as_bytes());
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

#[cfg(test)]
mod tests {
	use super::{
		expand_exec_env, is_exec_teardown_noise, map_exec_start_err, map_not_running, ExecOptions,
		EXEC_START_TIMEOUT,
	};

	#[test]
	fn expand_exec_env_passes_through_key_value() {
		let out = expand_exec_env(&["FOO=bar".to_string(), "BAZ=qux".to_string()]);
		assert_eq!(out, vec!["FOO=bar".to_string(), "BAZ=qux".to_string()]);
	}

	#[test]
	fn expand_exec_env_resolves_bare_key_from_host() {
		// A bare `KEY` takes its value from podup's own environment; an unset bare
		// key is dropped (libpod rejects a value-less env entry).
		std::env::set_var("PODUP_TEST_EXEC_ENV", "from-host");
		let out = expand_exec_env(&[
			"PODUP_TEST_EXEC_ENV".to_string(),
			"PODUP_TEST_EXEC_UNSET_ENV".to_string(),
		]);
		std::env::remove_var("PODUP_TEST_EXEC_ENV");
		assert_eq!(out, vec!["PODUP_TEST_EXEC_ENV=from-host".to_string()]);
	}

	#[test]
	fn teardown_noise_matches_only_connection_reset_frame() {
		assert!(is_exec_teardown_noise(
			"read unixpacket @->/run/...: read: connection reset by peer"
		));
		// Ordinary program output is never suppressed.
		assert!(!is_exec_teardown_noise("connection reset by peer"));
		assert!(!is_exec_teardown_noise("hello world"));
	}

	#[test]
	fn map_not_running_maps_404_and_stopped() {
		use crate::error::ComposeError;
		use crate::libpod::PodmanError;
		let e404 = PodmanError::Api {
			status: 404,
			message: "no such container: web".into(),
		};
		assert!(matches!(
			map_not_running(e404, "web"),
			ComposeError::NotRunning(s) if s == "web"
		));
		let e500 = PodmanError::Api {
			status: 500,
			message: "can only create exec sessions on running containers".into(),
		};
		assert!(matches!(
			map_not_running(e500, "web"),
			ComposeError::NotRunning(_)
		));
		// An unrelated error passes through unchanged.
		let other = PodmanError::Api {
			status: 500,
			message: "disk full".into(),
		};
		assert!(matches!(
			map_not_running(other, "web"),
			ComposeError::Podman(_)
		));
	}

	#[test]
	fn exec_start_timeout_with_user_names_the_user() {
		use crate::libpod::PodmanError;
		// A client-side head timeout (the wedged-launch symptom) becomes a clear,
		// fast ExecFailed naming the likely culprit — never the raw socket-timeout.
		let timeout = PodmanError::Api {
			status: 0,
			message: format!(
				"timed out after {}s waiting for the Podman socket to respond",
				EXEC_START_TIMEOUT.as_secs()
			),
		};
		let opts = ExecOptions {
			user: Some("doesnotexist".into()),
			..Default::default()
		};
		let mapped = map_exec_start_err(timeout, &opts);
		match mapped {
			crate::error::ComposeError::ExecFailed(msg) => {
				assert!(msg.contains("doesnotexist"), "got {msg}");
				assert!(msg.contains("did not start"), "got {msg}");
				assert!(
					!msg.to_ascii_lowercase().contains("podman socket"),
					"must not leak the socket-timeout wording: {msg}"
				);
			}
			other => panic!("expected ExecFailed, got {other:?}"),
		}
	}

	#[test]
	fn exec_start_timeout_without_user_names_the_workdir() {
		use crate::libpod::PodmanError;
		let timeout = PodmanError::Api {
			status: 0,
			message: "timed out after 20s waiting for the Podman socket to respond".into(),
		};
		let opts = ExecOptions {
			workdir: Some("/no/such/dir".into()),
			..Default::default()
		};
		match map_exec_start_err(timeout, &opts) {
			crate::error::ComposeError::ExecFailed(msg) => {
				assert!(msg.contains("/no/such/dir"), "got {msg}");
			}
			other => panic!("expected ExecFailed, got {other:?}"),
		}
	}

	#[test]
	fn exec_start_real_api_error_passes_through() {
		use crate::error::ComposeError;
		use crate::libpod::PodmanError;
		// The prompt HTTP error an engine returns for a bad user is a genuine
		// diagnostic and must reach the user verbatim, not be rewritten.
		let api = PodmanError::Api {
			status: 500,
			message: "unable to find user doesnotexist: no matching entries in passwd file".into(),
		};
		let opts = ExecOptions {
			user: Some("doesnotexist".into()),
			..Default::default()
		};
		match map_exec_start_err(api, &opts) {
			ComposeError::Podman(e) => {
				assert!(e.to_string().contains("no matching entries in passwd file"));
			}
			other => panic!("expected Podman passthrough, got {other:?}"),
		}
	}
}
