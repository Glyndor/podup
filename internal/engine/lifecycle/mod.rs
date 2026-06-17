//! Service lifecycle commands: up, down, start, stop, restart, kill, rm, pause, unpause, run.

mod commands;
mod targets;

use std::collections::{HashMap, HashSet};

use tracing::info;

use crate::compose::types::{ComposeFile, ServiceCondition};
use crate::error::Result;
use crate::libpod::API_PREFIX;

use targets::{expand_targets, filter_services};

use super::container::config_hash;

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
	/// Publish the service's declared `ports:` (compose `run --service-ports`).
	/// When false, `run` leaves ports unpublished to avoid host-port collisions.
	pub service_ports: bool,
}

impl Engine {
	/// Start all services defined in the compose file, creating containers that do not exist.
	pub async fn up(&self, file: &ComposeFile) -> Result<()> {
		self.up_with_options(file, false, &[], &[], false, false, false)
			.await
	}

	/// Start a container by name, ignoring an error from an already-running one.
	/// Used when `up` leaves an unchanged container in place but wants to ensure
	/// it is running.
	async fn ensure_started(&self, container_name: &str) {
		let path = format!(
			"{API_PREFIX}/containers/{}/start",
			crate::libpod::urlencoded(container_name)
		);
		let _ = self.client.post_empty_ok(&path).await;
	}

	/// Start services with explicit options. When `no_recreate` is true, running containers are left untouched. On partial failure, staging directories are cleaned up.
	#[allow(clippy::too_many_arguments)]
	pub async fn up_with_options(
		&self,
		file: &ComposeFile,
		_detach: bool,
		active_profiles: &[String],
		target_services: &[String],
		no_recreate: bool,
		force_recreate: bool,
		no_deps: bool,
	) -> Result<()> {
		async {
			let levels = crate::compose::resolve_levels(file)?;
			let active = active_profiles_set(active_profiles);

			let target_set = expand_targets(file, target_services, no_deps);

			// Prefetch the project's containers once (instead of one API call per
			// replica): which names already exist, and each one's config-hash
			// label so we can decide whether a container needs recreation.
			let mut present: HashSet<String> = HashSet::new();
			let mut existing_hash: HashMap<String, String> = HashMap::new();
			if !force_recreate {
				let filters = serde_json::json!({
					"label": [format!("podup.project={}", self.project)],
				});
				let path = format!(
					"{API_PREFIX}/containers/json?all=true&filters={}",
					crate::libpod::urlencoded(&filters.to_string()),
				);
				let entries = self
					.client
					.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
					.await
					.map_err(crate::error::ComposeError::Podman)?;
				for entry in entries {
					if let Some(hash) = entry.labels.get("podup.config-hash") {
						for raw in &entry.names {
							existing_hash
								.insert(raw.trim_start_matches('/').to_string(), hash.clone());
						}
					}
					for raw in entry.names {
						present.insert(raw.trim_start_matches('/').to_string());
					}
				}
			}

			self.create_networks(file).await?;
			self.create_volumes(file).await?;

			// Start each dependency level in turn; services within a level have
			// no `depends_on` relationship to each other (guaranteed by the
			// layering), so they start concurrently. The barrier between levels
			// preserves ordering and `service_healthy`/`service_completed`
			// semantics: a level only begins once the previous one is up.
			for level in &levels {
				let started = level.iter().map(|name| {
					self.up_one_service(
						name,
						file,
						&active,
						&target_set,
						&present,
						&existing_hash,
						no_recreate,
						force_recreate,
					)
				});
				futures_util::future::try_join_all(started).await?;
			}

			Ok(())
		}
		.await
	}

	/// Bring up a single service: honor profile/target filters, wait on its
	/// `depends_on` conditions, build or pull the image, and create/start each
	/// replica (skipping containers that are unchanged unless `force_recreate`).
	/// Used by [`Self::up_with_options`]; safe to run concurrently for services
	/// in the same dependency level (the `Engine` holds no per-call mutable
	/// state — the libpod client is connection-per-request).
	#[allow(clippy::too_many_arguments)]
	async fn up_one_service(
		&self,
		name: &str,
		file: &ComposeFile,
		active: &HashSet<String>,
		target_set: &Option<HashSet<String>>,
		present: &HashSet<String>,
		existing_hash: &HashMap<String, String>,
		no_recreate: bool,
		force_recreate: bool,
	) -> Result<()> {
		if let Some(set) = target_set {
			if !set.contains(name) {
				return Ok(());
			}
		}
		let service = &file.services[name];

		if !service_in_profiles(service, active) {
			tracing::debug!("skipping {name}: no active profile match");
			return Ok(());
		}

		for dep in service.depends_on.service_names() {
			let condition = service.depends_on.condition_for(&dep);
			// `required: false` makes the dependency optional — a failed wait
			// must not abort `up`, matching docker-compose v2.
			let required = service.depends_on.required_for(&dep);
			let dep_service = match file.services.get(&dep) {
				Some(s) => s,
				None => continue,
			};
			if !service_in_profiles(dep_service, active) {
				continue;
			}
			let dep_container = self.container_name(&dep, dep_service);

			let wait = match condition {
				ServiceCondition::ServiceStarted => Ok(()),
				ServiceCondition::ServiceHealthy => {
					// Wait unless the healthcheck is explicitly disabled in
					// compose. With no compose healthcheck we still wait:
					// `wait_healthy` consults the container's effective
					// healthcheck, so image-inherited ones are honored and
					// the wait short-circuits when none exists.
					let disabled = dep_service
						.healthcheck
						.as_ref()
						.is_some_and(|h| h.is_disabled());
					if disabled {
						tracing::debug!(
							"{dep} healthcheck disabled — skipping service_healthy wait"
						);
						Ok(())
					} else {
						self.wait_healthy(&dep_container, dep_service).await
					}
				}
				ServiceCondition::ServiceCompletedSuccessfully => {
					self.wait_completed(&dep_container).await
				}
			};
			match wait {
				Ok(()) => {}
				Err(e) if !required => {
					tracing::debug!(
						"optional dependency {dep} (required: false) did not satisfy its condition: {e}"
					);
				}
				Err(e) => return Err(e),
			}
		}

		let policy = service.pull_policy.as_deref().unwrap_or("missing");
		match (service.build.is_some(), policy) {
			(true, _) => {
				self.build_service(name, service, file, &crate::engine::BuildOptions::default())
					.await?
			}
			(false, "never") => {}
			(false, _) => self.pull_image(service).await?,
		}

		let replicas = service
			.scale
			.or(service.deploy.as_ref().and_then(|d| d.replicas))
			.unwrap_or(1) as usize;

		let new_hash = config_hash(service, file)?;

		for i in 1..=replicas {
			let container_name = if replicas == 1 {
				self.container_name(name, service)
			} else {
				format!("{}-{i}", self.container_name(name, service))
			};
			if !force_recreate {
				if no_recreate && present.contains(&container_name) {
					info!("{container_name} already exists — skipping recreate");
					self.ensure_started(&container_name).await;
					continue;
				}
				// Services with a build section are rebuilt on every up, so
				// their container must be recreated to pick up the fresh
				// image even when the compose config is unchanged.
				if service.build.is_none() && existing_hash.get(&container_name) == Some(&new_hash)
				{
					info!("{container_name} is up to date — skipping recreate");
					self.ensure_started(&container_name).await;
					continue;
				}
			}
			self.create_and_start(&container_name, name, service, file)
				.await?;
			info!("started {container_name}");

			for hook in &service.post_start {
				self.run_lifecycle_hook(&container_name, hook).await?;
			}
		}

		Ok(())
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

				let grace = self.grace_period_secs(service);
				let stop_path = format!(
					"{API_PREFIX}/containers/{}/stop?t={grace}",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.post_empty_ok(&stop_path).await {
					tracing::debug!("stop {container_name}: {e}");
				}

				let rm_path = format!(
					"{API_PREFIX}/containers/{}?force=true",
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
				"{API_PREFIX}/networks/{}",
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
					"{API_PREFIX}/volumes/{}",
					crate::libpod::urlencoded(&volume_name),
				);
				match self.client.delete_ok(&vol_path).await {
					Ok(_) => info!("removed volume {volume_name}"),
					Err(e) if e.is_status(404) => {}
					Err(e) => tracing::warn!("could not remove volume {volume_name}: {e}"),
				}
			}
		}

		// Internal native secrets are podup-owned (not user data), so remove
		// them unconditionally — independent of `remove_volumes`.
		self.remove_internal_secrets(file).await?;
		Ok(())
	}
}
