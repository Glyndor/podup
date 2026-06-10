//! Service lifecycle commands: up, down, start, stop, restart, kill, rm, pause, unpause, run.

use bollard::container::LogOutput;
use bollard::query_parameters::{
	KillContainerOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions,
	StopContainerOptions, WaitContainerOptions,
};
use futures::StreamExt;
use tracing::info;

use crate::compose::types::{ComposeFile, ServiceCondition};
use crate::error::{ComposeError, Result};

use super::profiles::{active_profiles_set, service_in_profiles};
use super::Engine;

/// Options for [`Engine::run`].
pub struct RunOptions {
	/// Override the default service command.
	pub cmd: Vec<String>,
	/// Remove the container after it exits.
	pub rm: bool,
	/// Start the container in the background without streaming logs.
	pub detach: bool,
	/// Additional environment variables (`KEY=VAL` strings, override service env).
	pub env_overrides: Vec<String>,
	/// Override the generated container name.
	pub name_override: Option<String>,
}

impl Engine {
	pub async fn up(&self, file: &ComposeFile) -> Result<()> {
		self.up_with_options(file, false, &[], &[], false).await
	}

	pub async fn up_with_options(
		&self,
		file: &ComposeFile,
		_detach: bool,
		active_profiles: &[String],
		target_services: &[String],
		no_recreate: bool,
	) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let active = active_profiles_set(active_profiles);

		let target_set: Option<std::collections::HashSet<String>> = if target_services.is_empty() {
			None
		} else {
			let mut set = std::collections::HashSet::new();
			let mut stack: Vec<String> = target_services.to_vec();
			while let Some(name) = stack.pop() {
				if !set.insert(name.clone()) {
					continue;
				}
				if let Some(service) = file.services.get(&name) {
					for dep in service.depends_on.service_names() {
						if !set.contains(&dep) {
							stack.push(dep);
						}
					}
				}
			}
			Some(set)
		};

		self.create_networks(file).await?;
		self.create_volumes(file).await?;

		for name in &order {
			if let Some(ref set) = target_set {
				if !set.contains(name) {
					continue;
				}
			}
			let service = &file.services[name];

			if !service_in_profiles(service, &active) {
				tracing::debug!("skipping {name}: no active profile match");
				continue;
			}

			for dep in service.depends_on.service_names() {
				let condition = service.depends_on.condition_for(&dep);
				let dep_service = match file.services.get(&dep) {
					Some(s) => s,
					None => continue,
				};
				if !service_in_profiles(dep_service, &active) {
					continue;
				}
				let dep_container = self.container_name(&dep, dep_service);

				match condition {
					ServiceCondition::ServiceStarted => {}
					ServiceCondition::ServiceHealthy => {
						if dep_service
							.healthcheck
							.as_ref()
							.map(|h| !h.is_disabled())
							.unwrap_or(false)
						{
							self.wait_healthy(&dep_container, dep_service).await?;
						} else {
							tracing::debug!(
								"{dep} requested service_healthy but has no healthcheck — skipping wait"
							);
						}
					}
					ServiceCondition::ServiceCompletedSuccessfully => {
						self.wait_completed(&dep_container).await?;
					}
				}
			}

			let policy = service.pull_policy.as_deref().unwrap_or("missing");
			match (service.build.is_some(), policy) {
				(true, _) => self.build_service(name, service).await?,
				(false, "never") => {}
				(false, _) => self.pull_image(service).await?,
			}

			let replicas = service
				.scale
				.or(service.deploy.as_ref().and_then(|d| d.replicas))
				.unwrap_or(1) as usize;

			for i in 1..=replicas {
				let container_name = if replicas == 1 {
					self.container_name(name, service)
				} else {
					format!("{}-{i}", self.container_name(name, service))
				};
				if no_recreate && self.is_container_running(&container_name).await {
					info!("{container_name} already running — skipping recreate");
					continue;
				}
				self.create_and_start(&container_name, name, service, file)
					.await?;
				self.connect_extra_networks(&container_name, service, file)
					.await?;
				info!("started {container_name}");

				for hook in &service.post_start {
					self.run_lifecycle_hook(&container_name, hook).await?;
				}
			}
		}

		Ok(())
	}

	pub async fn down(&self, file: &ComposeFile) -> Result<()> {
		self.down_with_options(file, false).await
	}

	pub async fn down_with_options(&self, file: &ComposeFile, remove_volumes: bool) -> Result<()> {
		let mut order = crate::compose::resolve_order(file)?;
		order.reverse();

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				for hook in &service.pre_stop {
					let _ = self.run_lifecycle_hook(&container_name, hook).await;
				}

				let _ = self
					.docker
					.stop_container(
						&container_name,
						Some(StopContainerOptions {
							t: Some(10),
							..Default::default()
						}),
					)
					.await;

				let _ = self
					.docker
					.remove_container(
						&container_name,
						Some(RemoveContainerOptions {
							force: true,
							v: remove_volumes,
							..Default::default()
						}),
					)
					.await;

				info!("removed {container_name}");
			}
		}

		self.cleanup_temp_dir();
		Ok(())
	}

	pub async fn restart(&self, file: &ComposeFile, service_name: Option<&str>) -> Result<()> {
		let names: Vec<String> = if let Some(svc) = service_name {
			if !file.services.contains_key(svc) {
				return Err(ComposeError::ServiceNotFound(svc.into()));
			}
			vec![svc.to_string()]
		} else {
			file.services.keys().cloned().collect()
		};

		for name in &names {
			let service = &file.services[name];
			let container_name = self.container_name(name, service);

			let _ = self
				.docker
				.stop_container(
					&container_name,
					Some(StopContainerOptions {
						t: Some(10),
						..Default::default()
					}),
				)
				.await;

			self.docker
				.start_container(&container_name, None::<StartContainerOptions>)
				.await?;

			info!("restarted {container_name}");

			for (dep_name, dep_service) in &file.services {
				if dep_service.depends_on.restart_for(name) {
					let dep_container = self.container_name(dep_name, dep_service);
					let _ = self
						.docker
						.stop_container(
							&dep_container,
							Some(StopContainerOptions {
								t: Some(10),
								..Default::default()
							}),
						)
						.await;
					if let Err(e) = self
						.docker
						.start_container(&dep_container, None::<StartContainerOptions>)
						.await
					{
						tracing::warn!("cascade restart of {dep_name} failed: {e}");
					} else {
						info!("cascade-restarted {dep_container} (depends_on.restart)");
					}
				}
			}
		}

		Ok(())
	}

	/// Stop running containers without removing them.
	///
	/// Services are stopped in reverse dependency order. If `target_services`
	/// is empty, all services in the compose file are stopped.
	pub async fn stop(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let mut order = crate::compose::resolve_order(file)?;
		order.reverse();
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				let _ = self
					.docker
					.stop_container(
						&container_name,
						Some(StopContainerOptions {
							t: Some(10),
							..Default::default()
						}),
					)
					.await;
				info!("stopped {container_name}");
			}
		}
		Ok(())
	}

	/// Start stopped containers.
	///
	/// Services are started in dependency order. If `target_services` is empty,
	/// all services in the compose file are started.
	pub async fn start(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				self.docker
					.start_container(&container_name, None::<StartContainerOptions>)
					.await?;
				info!("started {container_name}");
			}
		}
		Ok(())
	}

	/// Send a signal to service containers (default: `SIGKILL`).
	///
	/// If `target_services` is empty, all services are signalled.
	pub async fn kill(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		signal: &str,
	) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				self.docker
					.kill_container(
						&container_name,
						Some(KillContainerOptions {
							signal: signal.to_string(),
						}),
					)
					.await?;
				info!("sent {signal} to {container_name}");
			}
		}
		Ok(())
	}

	/// Remove stopped service containers.
	///
	/// When `force` is true, running containers are stopped before removal.
	/// Services are removed in reverse dependency order.
	pub async fn rm(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		force: bool,
	) -> Result<()> {
		let mut order = crate::compose::resolve_order(file)?;
		order.reverse();
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				let _ = self
					.docker
					.remove_container(
						&container_name,
						Some(RemoveContainerOptions {
							force,
							..Default::default()
						}),
					)
					.await;
				info!("removed {container_name}");
			}
		}
		Ok(())
	}

	/// Pause running service containers (SIGSTOP).
	///
	/// If `target_services` is empty, all services are paused.
	pub async fn pause(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				self.docker.pause_container(&container_name).await?;
				info!("paused {container_name}");
			}
		}
		Ok(())
	}

	/// Resume paused service containers.
	///
	/// If `target_services` is empty, all services are unpaused.
	pub async fn unpause(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				self.docker.unpause_container(&container_name).await?;
				info!("unpaused {container_name}");
			}
		}
		Ok(())
	}

	/// Run a one-off command in a new container for a service.
	///
	/// The container is started, its output streamed, and it is removed when done
	/// (unless `opts.rm` is false). Non-zero exit codes surface as `ComposeError::RunExited`.
	pub async fn run(
		&self,
		file: &ComposeFile,
		service_name: &str,
		opts: RunOptions,
	) -> Result<()> {
		let RunOptions {
			cmd,
			rm,
			detach,
			env_overrides,
			name_override,
		} = opts;
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;

		let run_name = name_override.unwrap_or_else(|| {
			format!("{}-{service_name}-run-{}", self.project, std::process::id())
		});

		let mut run_service = service.clone();
		if !cmd.is_empty() {
			run_service.command = Some(crate::compose::types::Command::Exec(cmd));
		}
		if !env_overrides.is_empty() {
			let mut env_list: Vec<String> = {
				let map = run_service.environment.to_map();
				map.into_iter()
					.map(|(k, v)| v.map_or(k.clone(), |v| format!("{k}={v}")))
					.collect()
			};
			env_list.extend(env_overrides);
			run_service.environment = crate::compose::types::EnvVars::List(env_list);
		}
		run_service.restart = None;

		self.create_and_start(&run_name, service_name, &run_service, file)
			.await?;

		if detach {
			info!("started run container {run_name}");
			return Ok(());
		}

		let mut log_stream = self.docker.logs(
			&run_name,
			Some(LogsOptions {
				stdout: true,
				stderr: true,
				follow: true,
				..Default::default()
			}),
		);

		while let Some(msg) = log_stream.next().await {
			match msg? {
				LogOutput::StdOut { message } => print!("{}", String::from_utf8_lossy(&message)),
				LogOutput::StdErr { message } => eprint!("{}", String::from_utf8_lossy(&message)),
				_ => {}
			}
		}

		let exit_code = {
			let mut wait_stream = self
				.docker
				.wait_container(&run_name, None::<WaitContainerOptions>);
			match wait_stream.next().await {
				Some(Ok(resp)) => resp.status_code,
				Some(Err(bollard::errors::Error::DockerContainerWaitError { code, .. })) => code,
				Some(Err(e)) => return Err(crate::error::ComposeError::Podman(e)),
				None => 0,
			}
		};

		if rm {
			let _ = self
				.docker
				.remove_container(
					&run_name,
					Some(RemoveContainerOptions {
						force: true,
						..Default::default()
					}),
				)
				.await;
		}

		if exit_code != 0 {
			return Err(crate::error::ComposeError::RunExited(exit_code));
		}

		Ok(())
	}
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the ordered service names filtered to `target_services`.
///
/// Returns an error if any name in `target_services` is not in the file.
fn filter_services(
	file: &crate::compose::types::ComposeFile,
	order: Vec<String>,
	target_services: &[String],
) -> Result<Vec<String>> {
	if target_services.is_empty() {
		return Ok(order);
	}
	for name in target_services {
		if !file.services.contains_key(name) {
			return Err(ComposeError::ServiceNotFound(name.clone()));
		}
	}
	let set: std::collections::HashSet<&str> = target_services.iter().map(|s| s.as_str()).collect();
	Ok(order
		.into_iter()
		.filter(|n| set.contains(n.as_str()))
		.collect())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::filter_services;
	use crate::compose::types::{ComposeFile, Service};

	fn file_with_services(names: &[&str]) -> ComposeFile {
		let mut file = ComposeFile::default();
		for &name in names {
			file.services.insert(name.to_string(), Service::default());
		}
		file
	}

	#[test]
	fn filter_empty_target_returns_all() {
		let file = file_with_services(&["a", "b", "c"]);
		let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
		let result = filter_services(&file, order.clone(), &[]).unwrap();
		assert_eq!(result, order);
	}

	#[test]
	fn filter_target_subset_returns_intersection() {
		let file = file_with_services(&["a", "b", "c"]);
		let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
		let result = filter_services(&file, order, &["b".to_string()]).unwrap();
		assert_eq!(result, vec!["b".to_string()]);
	}

	#[test]
	fn filter_target_preserves_order() {
		let file = file_with_services(&["a", "b", "c"]);
		let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
		let result = filter_services(&file, order, &["c".to_string(), "a".to_string()]).unwrap();
		assert_eq!(result, vec!["a".to_string(), "c".to_string()]);
	}

	#[test]
	fn filter_unknown_service_returns_error() {
		let file = file_with_services(&["a"]);
		let order = vec!["a".to_string()];
		let err = filter_services(&file, order, &["z".to_string()]).unwrap_err();
		assert!(matches!(
			err,
			crate::error::ComposeError::ServiceNotFound(_)
		));
	}
}
