//! Container orchestration engine.
//!
//! Translates a parsed [`ComposeFile`](crate::compose::types::ComposeFile) into Podman API calls via the libpod REST API.

mod build;
mod container;
mod copy;
mod events;
mod image;
pub use build::{BuildOptions, PullOptions, PushOptions};
pub use copy::CpOptions;
pub use image::resolve_image_digests;
pub use lifecycle::{RunOptions, RunOverrides};
pub use lock::ProjectLock;
pub use query::{ExecOptions, ImagesOptions, LogsOptions, PsOptions};
mod container_config;
mod health;
mod lifecycle;
mod lock;
mod network;
mod profiles;
mod projects;
pub use projects::{list_projects, LsOptions};
mod query;
mod secrets;
mod staging;
mod stats;
pub use staging::is_safe_project_name;
mod volume;
pub use volume::VolumesOptions;
mod volume_mounts;
#[cfg(feature = "watch")]
mod watch;

use std::io::Write;
use std::path::PathBuf;

use futures_util::StreamExt;

use crate::compose::types::{LifecycleHook, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::exec::{ExecCreateConfig, ExecStartConfig};
use crate::libpod::{Client, LogOutput, API_PREFIX};

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// Handle through which all Podman operations for a project are dispatched.
pub struct Engine {
	pub(super) client: Client,
	pub(super) project: String,
	pub(super) base_dir: PathBuf,
	/// Optional CLI `-t/--timeout` override (seconds) for container shutdown
	/// grace; when set it takes precedence over each service's
	/// `stop_grace_period`. `None` falls back to the per-service value.
	pub(super) stop_timeout: Option<i32>,
	/// CLI `--scale SERVICE=N` overrides (from `up --scale` and the `scale`
	/// subcommand); when a service is present it takes precedence over the
	/// compose `scale:`/`deploy.replicas` value. Empty falls back to compose.
	pub(super) scale_overrides: std::collections::HashMap<String, u32>,
	/// CLI `up --pull <policy>` override; takes precedence over each service's
	/// `pull_policy`. `None` falls back to the per-service value.
	pub(super) pull_policy_override: Option<String>,
	/// CLI `up --no-build`: never build images, even for services with a
	/// `build:` section (they fall back to pulling/using an existing image).
	pub(super) no_build: bool,
	/// CLI `up --quiet-pull`: suppress image-pull progress output.
	pub(super) quiet_pull: bool,
	/// CLI `run`-only flag overrides (user/workdir/entrypoint/volume/publish/
	/// interactive/no-deps); empty by default.
	pub(super) run_overrides: lifecycle::RunOverrides,
	/// CLI `up -V/--renew-anon-volumes`: when recreating a container, also remove
	/// its old anonymous volumes instead of leaving them orphaned.
	pub(super) renew_anon_volumes: bool,
}

impl Engine {
	/// Create an engine for `project_name` using the working directory as the base path for relative volume mounts.
	pub fn new(client: Client, project: String) -> Self {
		Self {
			client,
			project,
			base_dir: std::env::current_dir().unwrap_or_default(),
			stop_timeout: None,
			scale_overrides: std::collections::HashMap::new(),
			pull_policy_override: None,
			no_build: false,
			quiet_pull: false,
			run_overrides: lifecycle::RunOverrides::default(),
			renew_anon_volumes: false,
		}
	}

	/// Create an engine with an explicit base directory — use when the compose file is not in the working directory.
	pub fn with_base_dir(client: Client, project: String, base_dir: PathBuf) -> Self {
		Self {
			client,
			project,
			base_dir,
			stop_timeout: None,
			scale_overrides: std::collections::HashMap::new(),
			pull_policy_override: None,
			no_build: false,
			quiet_pull: false,
			run_overrides: lifecycle::RunOverrides::default(),
			renew_anon_volumes: false,
		}
	}

	/// Set the CLI `-t/--timeout` shutdown-grace override (seconds). Builder-style.
	pub fn with_stop_timeout(mut self, timeout: Option<i32>) -> Self {
		self.stop_timeout = timeout;
		self
	}

	/// Set the CLI `--scale SERVICE=N` replica overrides. Builder-style.
	pub fn with_scale_overrides(
		mut self,
		overrides: std::collections::HashMap<String, u32>,
	) -> Self {
		self.scale_overrides = overrides;
		self
	}

	/// Set the CLI `up` image-acquisition overrides: `--pull <policy>`,
	/// `--no-build`, and `--quiet-pull`. Builder-style.
	pub fn with_up_overrides(
		mut self,
		pull_policy: Option<String>,
		no_build: bool,
		quiet_pull: bool,
	) -> Self {
		self.pull_policy_override = pull_policy;
		self.no_build = no_build;
		self.quiet_pull = quiet_pull;
		self
	}

	/// Set the CLI `run`-only flag overrides (`-u/-w/--entrypoint/-v/-p/-i/
	/// --no-deps`). Builder-style; consumed by [`Engine::run`].
	pub fn with_run_overrides(mut self, overrides: RunOverrides) -> Self {
		self.run_overrides = overrides;
		self
	}

	/// Set the CLI `up -V/--renew-anon-volumes` flag. Builder-style; when set,
	/// recreating a container also removes its old anonymous volumes.
	pub fn with_renew_anon_volumes(mut self, renew: bool) -> Self {
		self.renew_anon_volumes = renew;
		self
	}

	/// Resolve the replica count for a service: a CLI `--scale` override wins,
	/// else the compose `scale:`, else `deploy.replicas`, else 1. The single
	/// source of truth so `up`, naming, and teardown never drift.
	pub(super) fn resolve_replicas(&self, service_name: &str, service: &Service) -> usize {
		if let Some(&n) = self.scale_overrides.get(service_name) {
			return n as usize;
		}
		service
			.scale
			.or(service.deploy.as_ref().and_then(|d| d.replicas))
			.unwrap_or(1) as usize
	}

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
		// Lock stdout once for the whole stream instead of re-acquiring the lock
		// (and issuing a syscall) per frame; stdout is ours exclusively on this
		// path. stderr is locked per frame because the tracing subscriber also
		// writes there: holding its lock across the await loop would starve
		// concurrent log emissions. Flush after each frame so output stays prompt.
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

	pub(super) fn container_name(&self, service_name: &str, service: &Service) -> String {
		service
			.container_name
			.clone()
			.unwrap_or_else(|| format!("{}-{}", self.project, service_name))
	}

	pub(super) fn replica_names(&self, service_name: &str, service: &Service) -> Vec<String> {
		let replicas = self.resolve_replicas(service_name, service);
		let base = self.container_name(service_name, service);
		if replicas <= 1 {
			vec![base]
		} else {
			(1..=replicas).map(|i| format!("{base}-{i}")).collect()
		}
	}

	pub(super) fn first_replica_name(&self, service_name: &str, service: &Service) -> String {
		let replicas = self.resolve_replicas(service_name, service);
		let base = self.container_name(service_name, service);
		if replicas <= 1 {
			base
		} else {
			format!("{base}-1")
		}
	}

	/// Resolve the container name for a service replica: the 1-based `--index`
	/// when given (erroring if out of range), else the first replica. Shared by
	/// the replica-targeting commands (`exec`, `cp`).
	pub(super) fn replica_name_at(
		&self,
		service_name: &str,
		service: &Service,
		index: Option<u32>,
	) -> Result<String> {
		match index {
			Some(i) => {
				// `--index` is 1-based; `0` is an invalid index, not "first replica".
				let idx = (i as usize).checked_sub(1).ok_or_else(|| {
					ComposeError::ServiceNotFound(format!(
						"{service_name} (replica index {i}: indexes are 1-based)"
					))
				})?;
				let names = self.replica_names(service_name, service);
				names.get(idx).cloned().ok_or_else(|| {
					ComposeError::ServiceNotFound(format!("{service_name} (replica index {i})"))
				})
			}
			None => Ok(self.first_replica_name(service_name, service)),
		}
	}

	/// Watch for file changes and apply the service's `develop.watch` rules. Returns an error when the `watch` feature is disabled.
	#[cfg(not(feature = "watch"))]
	pub async fn watch(&self, _file: &crate::compose::types::ComposeFile) -> Result<()> {
		Err(crate::error::ComposeError::Unsupported(
			"watch requires the 'watch' feature".into(),
		))
	}
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

fn walk_dir(root: &std::path::Path) -> std::io::Result<Vec<PathBuf>> {
	let mut out = Vec::new();
	walk_collect(root, &mut out)?;
	Ok(out)
}

fn walk_collect(dir: &std::path::Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
	let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
	entries.sort_by_key(|e| e.file_name());
	for entry in entries {
		let path = entry.path();
		let file_type = entry.file_type()?;
		out.push(path.clone());
		if file_type.is_dir() {
			walk_collect(&path, out)?;
		}
	}
	Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::libpod::Client;

	fn engine(project: &str) -> Engine {
		Engine::with_base_dir(
			Client::new("/nonexistent.sock"),
			project.into(),
			std::env::temp_dir(),
		)
	}

	fn scaled_service(replicas: u32) -> Service {
		Service {
			scale: Some(replicas),
			..Service::default()
		}
	}

	#[test]
	fn replica_name_at_index_zero_is_rejected() {
		// `--index` is 1-based; index 0 must be an error, never replica 1.
		let e = engine("proj");
		let svc = scaled_service(3);
		let err = e
			.replica_name_at("web", &svc, Some(0))
			.expect_err("index 0 must be rejected");
		assert!(
			matches!(err, ComposeError::ServiceNotFound(ref m) if m.contains("1-based")),
			"unexpected error: {err:?}"
		);
	}

	#[test]
	fn replica_name_at_index_one_is_first_replica() {
		let e = engine("proj");
		let svc = scaled_service(3);
		assert_eq!(
			e.replica_name_at("web", &svc, Some(1)).unwrap(),
			"proj-web-1"
		);
	}

	#[test]
	fn replica_name_at_index_n_is_nth_replica() {
		let e = engine("proj");
		let svc = scaled_service(3);
		assert_eq!(
			e.replica_name_at("web", &svc, Some(3)).unwrap(),
			"proj-web-3"
		);
	}

	#[test]
	fn replica_name_at_out_of_range_is_rejected() {
		let e = engine("proj");
		let svc = scaled_service(3);
		assert!(e.replica_name_at("web", &svc, Some(4)).is_err());
	}

	#[test]
	fn replica_name_at_none_is_first_replica() {
		let e = engine("proj");
		// Single replica: the unsuffixed container name.
		assert_eq!(
			e.replica_name_at("web", &Service::default(), None).unwrap(),
			"proj-web"
		);
		// Multiple replicas: the first suffixed name.
		assert_eq!(
			e.replica_name_at("web", &scaled_service(3), None).unwrap(),
			"proj-web-1"
		);
	}
}
