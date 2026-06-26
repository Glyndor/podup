//! Service lifecycle commands: up, down, start, stop, restart, kill, rm, pause, unpause, run.

mod commands;
mod scale;
mod targets;

use std::collections::{HashMap, HashSet};

use tracing::info;

use crate::compose::types::{ComposeFile, ServiceCondition};
use crate::error::Result;
use crate::libpod::API_PREFIX;

use targets::{expand_targets, filter_services, in_started_set};

use super::container::config_hash;

use super::network::resolve_network_name;
use super::profiles::{active_profiles_set, service_in_profiles};
use super::Engine;

/// Options for [`Engine::run`].
#[derive(Default)]
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

/// Extra `docker compose run` flag overrides threaded through the engine
/// builder ([`Engine::with_run_overrides`]). These are CLI-only refinements of
/// a `run` invocation; they live here rather than on the frozen [`RunOptions`]
/// public struct so the 1.0 library API stays stable, mirroring how the `up`
/// image-acquisition overrides are carried on the engine.
#[derive(Default, Clone)]
pub struct RunOverrides {
	/// Run the command as this user (`-u/--user`, `name or UID[:GID]`).
	pub user: Option<String>,
	/// Working directory inside the container (`-w/--workdir`).
	pub workdir: Option<String>,
	/// Override the image entrypoint (`--entrypoint`).
	pub entrypoint: Option<String>,
	/// Extra ad-hoc volume mounts in compose short form (`-v/--volume`).
	pub volumes: Vec<String>,
	/// Extra published ports in compose short form (`-p/--publish`).
	pub publish: Vec<String>,
	/// Keep STDIN open on the container (`-i/--interactive`).
	pub interactive: bool,
	/// Do not start `depends_on` services before the run (`--no-deps`).
	pub no_deps: bool,
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
		self.run_up(
			file,
			active_profiles,
			target_services,
			no_recreate,
			force_recreate,
			no_deps,
			true,
		)
		.await
	}

	/// Create containers for services without starting them (docker compose
	/// `create`). Shares the `up` path with `start = false`: images are built/
	/// pulled and containers created, but never started, and no `depends_on`
	/// waits or `post_start` hooks run (nothing is running to gate on).
	#[allow(clippy::too_many_arguments)]
	pub async fn create_with_options(
		&self,
		file: &ComposeFile,
		active_profiles: &[String],
		target_services: &[String],
		no_recreate: bool,
		force_recreate: bool,
		no_deps: bool,
	) -> Result<()> {
		self.run_up(
			file,
			active_profiles,
			target_services,
			no_recreate,
			force_recreate,
			no_deps,
			false,
		)
		.await
	}

	#[allow(clippy::too_many_arguments)]
	async fn run_up(
		&self,
		file: &ComposeFile,
		active_profiles: &[String],
		target_services: &[String],
		no_recreate: bool,
		force_recreate: bool,
		no_deps: bool,
		start: bool,
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
			// Pre-create the union of inline secrets/configs once, before the
			// concurrent per-level start loop, so two services in the same level
			// can't race the non-atomic delete-then-create of a shared name.
			self.create_inline_secrets(file).await?;

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
						start,
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
		start: bool,
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

		// `create` (start = false) only builds the containers, so there is nothing
		// to gate on — skip the `depends_on` readiness waits entirely.
		for dep in service
			.depends_on
			.service_names()
			.into_iter()
			.filter(|_| start)
		{
			// Under `--no-deps` (and partial target lists) a dependency may have
			// been intentionally excluded from the started set. docker-compose
			// skips its readiness condition in that case; matching that avoids
			// waiting on (and 404-ing against) a container that was never
			// created.
			if !in_started_set(target_set, &dep) {
				tracing::debug!("{dep} not in started target set — skipping {name} readiness wait");
				continue;
			}

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
			// Scaled dep has no base-named container; wait on its first replica.
			let dep_container = self.first_replica_name(&dep, dep_service);

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

		// `up --pull <policy>` overrides the per-service `pull_policy`; `--no-build`
		// suppresses building even for services with a `build:` section (they fall
		// back to pulling/using an existing image).
		let policy = self
			.pull_policy_override
			.as_deref()
			.or(service.pull_policy.as_deref())
			.unwrap_or("missing");
		match (service.build.is_some() && !self.no_build, policy) {
			(true, _) => {
				self.build_service(name, service, file, &crate::engine::BuildOptions::default())
					.await?
			}
			(false, "never") => {}
			(false, _) => self.pull_image(service).await?,
		}

		let replicas = self.resolve_replicas(name, service);
		// A scaled service that publishes a fixed host port cannot start: only
		// one container can bind it. Fail fast with guidance instead of letting
		// replicas 2..N die mid-up with `address already in use`.
		scale::check_scale_port_conflict(name, service, replicas)?;

		let new_hash = config_hash(service, file)?;

		for i in 1..=replicas {
			let container_name = if replicas <= 1 {
				self.container_name(name, service)
			} else {
				format!("{}-{i}", self.container_name(name, service))
			};
			if !force_recreate {
				if no_recreate && present.contains(&container_name) {
					info!("{container_name} already exists — skipping recreate");
					// `create` leaves an existing container as-is; `up` ensures it runs.
					if start {
						self.ensure_started(&container_name).await;
					}
					continue;
				}
				// Services with a build section are rebuilt on every up, so
				// their container must be recreated to pick up the fresh
				// image even when the compose config is unchanged.
				if service.build.is_none() && existing_hash.get(&container_name) == Some(&new_hash)
				{
					info!("{container_name} is up to date — skipping recreate");
					if start {
						self.ensure_started(&container_name).await;
					}
					continue;
				}
			}
			self.create_and_start(&container_name, name, service, file, start)
				.await?;
			info!(
				"{} {container_name}",
				if start { "started" } else { "created" }
			);

			// `post_start` hooks run inside a running container, so only on `up`.
			if start {
				for hook in &service.post_start {
					self.run_lifecycle_hook(&container_name, hook).await?;
				}
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

		// Prefetch every project container once and group by service, instead of
		// one container-list round-trip per service (S+1 → 1 for the ordered pass).
		let live_by_service = self.list_project_containers_by_service().await?;

		for name in &order {
			let service = &file.services[name];
			let container_names = match live_by_service.get(name) {
				Some(live) if !live.is_empty() => live.clone(),
				_ => self.replica_names(name, service),
			};
			for container_name in container_names {
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
					tracing::warn!("could not stop {container_name}: {e}");
				}

				let rm_path = format!(
					"{API_PREFIX}/containers/{}?force=true",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self.client.delete_ok(&rm_path).await {
					tracing::warn!("could not remove {container_name}: {e}");
				} else {
					info!("removed {container_name}");
				}
			}
		}

		// A prior `up --scale`/`scale` may have created replicas the compose
		// file's default count no longer names; sweep any remaining project
		// containers by label so teardown is always complete.
		let grace = self.stop_timeout.unwrap_or(10);
		for name in self.list_project_container_names(None).await? {
			self.stop_and_remove(&name, grace).await;
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

	/// Remove the images used by the project's services (`down --rmi`). With
	/// `local_only`, only images of services that build locally (a `build:`
	/// section) are removed — matching `docker compose down --rmi local`.
	pub async fn remove_service_images(&self, file: &ComposeFile, local_only: bool) -> Result<()> {
		for (name, service) in &file.services {
			let builds_locally = service.build.is_some();
			if local_only && !builds_locally {
				continue;
			}
			let image = match &service.image {
				Some(img) => img.clone(),
				// A build-only service's image defaults to `{service}:latest`.
				None if builds_locally => format!("{name}:latest"),
				None => continue,
			};
			let path = format!(
				"{API_PREFIX}/images/{}?force=true",
				crate::libpod::urlencoded(&image),
			);
			match self.client.delete_ok(&path).await {
				Ok(_) => info!("removed image {image}"),
				Err(e) if e.is_status(404) => {}
				Err(e) => tracing::warn!("could not remove image {image}: {e}"),
			}
		}
		Ok(())
	}
}
