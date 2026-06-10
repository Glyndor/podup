//! Container orchestration engine.
//!
//! Translates a parsed [`ComposeFile`] into Podman API calls via bollard.

mod build;
mod container;
mod copy;
pub use lifecycle::RunOptions;
mod container_config;
mod container_misc;
mod health;
mod lifecycle;
mod network;
mod profiles;
mod query;
mod staging;
mod volume;
mod volume_mounts;
#[cfg(feature = "watch")]
mod watch;

use std::path::PathBuf;

use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::Docker;
use futures::StreamExt;

use crate::compose::types::{LifecycleHook, Service};
use crate::error::Result;

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// Handle through which all Podman operations for a project are dispatched.
pub struct Engine {
	pub(super) docker: Docker,
	pub(super) project: String,
	pub(super) base_dir: PathBuf,
}

impl Engine {
	/// Create an engine for `project_name` using the working directory as the base path for relative volume mounts.
	pub fn new(docker: Docker, project: String) -> Self {
		Self {
			docker,
			project,
			base_dir: std::env::current_dir().unwrap_or_default(),
		}
	}

	/// Create an engine with an explicit base directory — use when the compose file is not in the working directory.
	pub fn with_base_dir(docker: Docker, project: String, base_dir: PathBuf) -> Self {
		Self {
			docker,
			project,
			base_dir,
		}
	}

	pub(super) async fn run_lifecycle_hook(
		&self,
		container_name: &str,
		hook: &LifecycleHook,
	) -> Result<()> {
		let cmd = hook.command.to_exec();
		let env: Option<Vec<String>> = {
			let m = hook.environment.to_map();
			if m.is_empty() {
				None
			} else {
				Some(
					m.into_iter()
						.filter_map(|(k, v)| v.map(|v| format!("{k}={v}")))
						.collect(),
				)
			}
		};

		let exec_id = self
			.docker
			.create_exec(
				container_name,
				CreateExecOptions::<String> {
					cmd: Some(cmd),
					user: hook.user.clone(),
					privileged: hook.privileged,
					working_dir: hook.working_dir.clone(),
					env,
					attach_stdout: Some(true),
					attach_stderr: Some(true),
					..Default::default()
				},
			)
			.await?
			.id;

		match self.docker.start_exec(&exec_id, None).await? {
			StartExecResults::Attached { mut output, .. } => {
				while let Some(msg) = output.next().await {
					match msg? {
						LogOutput::StdOut { message } => {
							print!("{}", String::from_utf8_lossy(&message));
						}
						LogOutput::StdErr { message } => {
							eprint!("{}", String::from_utf8_lossy(&message));
						}
						_ => {}
					}
				}
			}
			StartExecResults::Detached => {}
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
		let replicas = service
			.scale
			.or(service.deploy.as_ref().and_then(|d| d.replicas))
			.unwrap_or(1) as usize;
		let base = self.container_name(service_name, service);
		if replicas == 1 {
			vec![base]
		} else {
			(1..=replicas).map(|i| format!("{base}-{i}")).collect()
		}
	}

	pub(super) fn first_replica_name(&self, service_name: &str, service: &Service) -> String {
		let replicas = service
			.scale
			.or(service.deploy.as_ref().and_then(|d| d.replicas))
			.unwrap_or(1) as usize;
		let base = self.container_name(service_name, service);
		if replicas == 1 {
			base
		} else {
			format!("{base}-1")
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
