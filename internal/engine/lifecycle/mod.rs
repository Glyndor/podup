//! Service lifecycle commands: up, down, start, stop, restart, kill, rm, pause, unpause, run.

mod commands;

use std::collections::{HashMap, HashSet};

use tracing::info;

use crate::compose::types::{ComposeFile, Service, ServiceCondition};
use crate::error::{ComposeError, Result};
use crate::libpod::API_PREFIX;

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
		let r: Result<()> = async {
			let order = crate::compose::resolve_order(file)?;
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
							} else {
								self.wait_healthy(&dep_container, dep_service).await?;
							}
						}
						ServiceCondition::ServiceCompletedSuccessfully => {
							self.wait_completed(&dep_container).await?;
						}
					}
				}

				let policy = service.pull_policy.as_deref().unwrap_or("missing");
				match (service.build.is_some(), policy) {
					(true, _) => self.build_service(name, service, file).await?,
					(false, "never") => {}
					(false, _) => self.pull_image(service).await?,
				}

				let replicas = service
					.scale
					.or(service.deploy.as_ref().and_then(|d| d.replicas))
					.unwrap_or(1) as usize;

				let new_hash = config_hash(service);

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
						if service.build.is_none()
							&& existing_hash.get(&container_name) == Some(&new_hash)
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

		self.cleanup_temp_dir();
		Ok(())
	}
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn grace_period_secs(service: &Service) -> i32 {
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
pub(super) fn filter_services(
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

/// Resolve which services `up` should start given an explicit target list.
///
/// Returns `None` when no targets are given (start everything). Otherwise the
/// set contains the targets plus, unless `no_deps` is set, their transitive
/// `depends_on` services.
fn expand_targets(
	file: &ComposeFile,
	target_services: &[String],
	no_deps: bool,
) -> Option<HashSet<String>> {
	if target_services.is_empty() {
		return None;
	}
	let mut set = HashSet::new();
	let mut stack: Vec<String> = target_services.to_vec();
	while let Some(name) = stack.pop() {
		if !set.insert(name.clone()) {
			continue;
		}
		if !no_deps {
			if let Some(service) = file.services.get(&name) {
				for dep in service.depends_on.service_names() {
					if !set.contains(&dep) {
						stack.push(dep);
					}
				}
			}
		}
	}
	Some(set)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::{expand_targets, filter_services};
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

	// --- expand_targets ---

	fn file_web_depends_db() -> ComposeFile {
		crate::parse_str(
			"services:\n  db:\n    image: x\n  web:\n    image: x\n    depends_on:\n      - db\n",
		)
		.unwrap()
	}

	#[test]
	fn expand_targets_empty_is_none() {
		let file = file_web_depends_db();
		assert!(expand_targets(&file, &[], false).is_none());
	}

	#[test]
	fn expand_targets_includes_dependencies() {
		let file = file_web_depends_db();
		let set = expand_targets(&file, &["web".to_string()], false).unwrap();
		assert!(set.contains("web"));
		assert!(set.contains("db"));
	}

	#[test]
	fn expand_targets_no_deps_excludes_dependencies() {
		let file = file_web_depends_db();
		let set = expand_targets(&file, &["web".to_string()], true).unwrap();
		assert!(set.contains("web"));
		assert!(!set.contains("db"));
	}
}
