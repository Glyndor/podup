//! Service lifecycle commands: up, down, restart.

use bollard::query_parameters::{RemoveContainerOptions, StartContainerOptions, StopContainerOptions};
use tracing::info;

use crate::compose::types::{ComposeFile, ServiceCondition};
use crate::error::{ComposeError, Result};

use super::profiles::{active_profiles_set, service_in_profiles};
use super::Engine;

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
}
