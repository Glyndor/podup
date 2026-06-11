//! Service lifecycle commands: up, down, start, stop, restart, kill, rm, pause, unpause, run.

use std::collections::HashSet;

use futures::StreamExt;
use tracing::info;

use crate::compose::types::{ComposeFile, Service, ServiceCondition};
use crate::error::{ComposeError, Result};

use super::network::resolve_network_name;
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
	/// Start all services defined in the compose file, creating containers that do not exist.
	pub async fn up(&self, file: &ComposeFile) -> Result<()> {
		self.up_with_options(file, false, &[], &[], false).await
	}

	/// Start services with explicit options. When `no_recreate` is true, running containers are left untouched. On partial failure, staging directories are cleaned up.
	pub async fn up_with_options(
		&self,
		file: &ComposeFile,
		_detach: bool,
		active_profiles: &[String],
		target_services: &[String],
		no_recreate: bool,
	) -> Result<()> {
		let r: Result<()> = async {
			let order = crate::compose::resolve_order(file)?;
			let active = active_profiles_set(active_profiles);

			let target_set: Option<HashSet<String>> = if target_services.is_empty() {
				None
			} else {
				let mut set = HashSet::new();
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

			// prefetch running containers once instead of one API call per replica.
			let running: HashSet<String> = if no_recreate {
				let filters = serde_json::json!({
					"label": [format!("podup.project={}", self.project)],
				});
				let path = format!(
					"/libpod/containers/json?filters={}",
					crate::libpod::urlencoded(&filters.to_string()),
				);
				self.client
					.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
					.await
					.map_err(crate::error::ComposeError::Podman)?
					.into_iter()
					.flat_map(|c| c.names)
					.map(|n| n.trim_start_matches('/').to_string())
					.collect()
			} else {
				HashSet::new()
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
					if no_recreate && running.contains(&container_name) {
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
		.await;
		// clean up staging dir on partial failure so inline secret/config
		// files are not left behind when up errors mid-way.
		if r.is_err() {
			self.cleanup_temp_dir();
		}
		r
	}

	/// Stop and remove all containers for the project. Does not remove volumes unless `remove_volumes` is set.
	pub async fn down(&self, file: &ComposeFile) -> Result<()> {
		self.down_with_options(file, false).await
	}

	/// Stop and remove services in reverse dependency order. Optionally removes named volumes and orphaned containers.
	pub async fn down_with_options(&self, file: &ComposeFile, remove_volumes: bool) -> Result<()> {
		let mut order = crate::compose::resolve_order(file)?;
		order.reverse();

		for name in &order {
			let service = &file.services[name];
			for container_name in self.replica_names(name, service) {
				for hook in &service.pre_stop {
					if let Err(e) = self.run_lifecycle_hook(&container_name, hook).await {
						tracing::debug!("pre_stop hook {container_name}: {e}");
					}
				}

				let grace = grace_period_secs(service);
				let stop_path = format!(
					"/libpod/containers/{}/stop?t={grace}",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.post_empty_ok(&stop_path).await {
					tracing::debug!("stop {container_name}: {e}");
				}

				let rm_path = format!(
					"/libpod/containers/{}?force=true",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.delete_ok(&rm_path).await {
					tracing::debug!("down delete {container_name}: {e}");
				}

				info!("removed {container_name}");
			}
		}

		for (key, config) in &file.networks {
			let external = config.as_ref().and_then(|c| c.external).unwrap_or(false);
			if external {
				continue;
			}
			let network_name = resolve_network_name(key, file, &self.project);
			let net_path = format!(
				"/libpod/networks/{}",
				crate::libpod::urlencoded(&network_name),
			);
			match self.client.delete_ok(&net_path).await {
				Ok(_) => info!("removed network {network_name}"),
				Err(e) if e.is_status(404) => {}
				Err(e) => tracing::warn!("could not remove network {network_name}: {e}"),
			}
		}

		if remove_volumes {
			for (key, config) in &file.volumes {
				let external = config.as_ref().and_then(|c| c.external).unwrap_or(false);
				if external {
					continue;
				}
				let volume_name = config
					.as_ref()
					.and_then(|c| c.name.as_deref())
					.map(|s| s.to_string())
					.unwrap_or_else(|| format!("{}_{}", self.project, key));
				let vol_path = format!(
					"/libpod/volumes/{}",
					crate::libpod::urlencoded(&volume_name),
				);
				match self.client.delete_ok(&vol_path).await {
					Ok(_) => info!("removed volume {volume_name}"),
					Err(e) if e.is_status(404) => {}
					Err(e) => tracing::warn!("could not remove volume {volume_name}: {e}"),
				}
			}
		}

		self.cleanup_temp_dir();
		Ok(())
	}

	/// Restart the named service (or all services). Dependents with a `restart` condition in `depends_on` are also restarted.
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

			for container_name in self.replica_names(name, service) {
				let grace = grace_period_secs(service);
				let stop_path = format!(
					"/libpod/containers/{}/stop?t={grace}",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.post_empty_ok(&stop_path).await {
					tracing::debug!("stop before restart {container_name}: {e}");
				}

				let start_path = format!(
					"/libpod/containers/{}/start",
					crate::libpod::urlencoded(&container_name),
				);
				self.client.post_empty_ok(&start_path).await.map_err(ComposeError::Podman)?;

				info!("restarted {container_name}");
			}

			for (dep_name, dep_service) in &file.services {
				if dep_service.depends_on.restart_for(name) {
					for dep_container in self.replica_names(dep_name, dep_service) {
						let grace = grace_period_secs(dep_service);
						let stop_path = format!(
							"/libpod/containers/{}/stop?t={grace}",
							crate::libpod::urlencoded(&dep_container),
						);
						if let Err(e) = self.client.post_empty_ok(&stop_path).await {
							tracing::debug!("stop before cascade restart {dep_container}: {e}");
						}
						let start_path = format!(
							"/libpod/containers/{}/start",
							crate::libpod::urlencoded(&dep_container),
						);
						if let Err(e) = self.client.post_empty_ok(&start_path).await {
							tracing::warn!("cascade restart of {dep_name} failed: {e}");
						} else {
							info!("cascade-restarted {dep_container} (depends_on.restart)");
						}
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
				let grace = grace_period_secs(service);
				let path = format!(
					"/libpod/containers/{}/stop?t={grace}",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.post_empty_ok(&path).await {
					tracing::debug!("stop {container_name}: {e}");
				}
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
				let path = format!(
					"/libpod/containers/{}/start",
					crate::libpod::urlencoded(&container_name),
				);
				self.client.post_empty_ok(&path).await.map_err(ComposeError::Podman)?;
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
				let path = format!(
					"/libpod/containers/{}/kill?signal={}",
					crate::libpod::urlencoded(&container_name),
					crate::libpod::urlencoded(signal),
				);
				self.client.post_empty_ok(&path).await.map_err(ComposeError::Podman)?;
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
				let force_str = if force { "true" } else { "false" };
				let path = format!(
					"/libpod/containers/{}?force={force_str}",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.delete_ok(&path).await {
					tracing::debug!("rm {container_name}: {e}");
				}
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
				let path = format!(
					"/libpod/containers/{}/pause",
					crate::libpod::urlencoded(&container_name),
				);
				self.client.post_empty_ok(&path).await.map_err(ComposeError::Podman)?;
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
				let path = format!(
					"/libpod/containers/{}/unpause",
					crate::libpod::urlencoded(&container_name),
				);
				self.client.post_empty_ok(&path).await.map_err(ComposeError::Podman)?;
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
		// Force non-TTY so Podman uses multiplexed log framing that
		// parse_multiplexed can decode. TTY mode sends raw bytes without
		// the 8-byte header, which would produce garbled output.
		run_service.tty = None;

		self.create_and_start(&run_name, service_name, &run_service, file)
			.await?;

		if detach {
			info!("started run container {run_name}");
			return Ok(());
		}

		let logs_path = format!(
			"/libpod/containers/{}/logs?follow=true&stdout=true&stderr=true",
			crate::libpod::urlencoded(&run_name),
		);
		let logs_resp = self
			.client
			.get_stream(&logs_path)
			.await
			.map_err(ComposeError::Podman)?;
		let mut log_stream = crate::libpod::parse_multiplexed(logs_resp.into_body());

		while let Some(msg) = log_stream.next().await {
			match msg.map_err(ComposeError::Podman)? {
				crate::libpod::LogOutput::StdOut { message } => {
					print!("{}", String::from_utf8_lossy(&message))
				}
				crate::libpod::LogOutput::StdErr { message } => {
					eprint!("{}", String::from_utf8_lossy(&message))
				}
			}
		}

		let wait_path = format!(
			"/libpod/containers/{}/wait?condition=stopped",
			crate::libpod::urlencoded(&run_name),
		);
		let exit_code = match self
			.client
			.post_empty_json::<crate::libpod::types::container::WaitResponse>(&wait_path)
			.await
		{
			Ok(resp) => {
				if let Some(msg) = resp.error.and_then(|e| e.message).filter(|m| !m.is_empty()) {
					tracing::warn!("container wait error: {msg}");
				}
				resp.status_code
			}
			Err(e) => {
				tracing::warn!("wait failed: {e}");
				0
			}
		};

		if rm {
			let rm_path = format!(
				"/libpod/containers/{}?force=true",
				crate::libpod::urlencoded(&run_name),
			);
			if let Err(e) = self.client.delete_ok(&rm_path).await {
				tracing::debug!("run cleanup delete {run_name}: {e}");
			}
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

fn grace_period_secs(service: &Service) -> i32 {
	service
		.stop_grace_period
		.as_deref()
		.and_then(crate::size::parse_duration_secs)
		.and_then(|s| i32::try_from(s).ok())
		.unwrap_or(10)
}

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
