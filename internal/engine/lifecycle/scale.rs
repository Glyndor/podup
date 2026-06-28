//! The `scale` subcommand plus the replica-listing/reconciliation helpers
//! shared with teardown.

use std::collections::HashSet;

use tracing::info;

use crate::compose::types::{ComposeFile, Service};
use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

/// The default ceiling on a service's replica count.
const DEFAULT_MAX_REPLICAS: u32 = 256;

/// The replica ceiling, overridable via the `PODUP_MAX_REPLICAS` environment
/// variable (a host operator's escape hatch). A missing, unparseable, or zero
/// override falls back to [`DEFAULT_MAX_REPLICAS`].
fn max_replicas() -> u32 {
	std::env::var("PODUP_MAX_REPLICAS")
		.ok()
		.and_then(|v| v.parse::<u32>().ok())
		.filter(|&n| n > 0)
		.unwrap_or(DEFAULT_MAX_REPLICAS)
}

/// Reject a replica count beyond the configured ceiling. Guards both the CLI
/// `scale`/`--scale` path and an untrusted compose `deploy.replicas`/`scale:`
/// from driving podup into unbounded container creation (a host DoS), since
/// every command resolves its replica count through this one check.
pub(super) fn check_replica_limit(service_name: &str, replicas: usize) -> Result<()> {
	let max = max_replicas();
	if replicas as u64 > u64::from(max) {
		return Err(ComposeError::ReplicaLimitExceeded {
			service: service_name.to_string(),
			replicas,
			max,
		});
	}
	Ok(())
}

/// Reject a scaled service that publishes a fixed host port: only one container
/// can bind a given host port, so replicas 2..N would fail at runtime with
/// `address already in use`. A host port of 0/None is runtime-assigned by
/// Podman, so such a service scales fine. The compose-spec does not define how
/// scaling interacts with published ports, so podup fails fast rather than
/// inventing surprising auto-offset semantics.
pub(super) fn check_scale_port_conflict(
	service_name: &str,
	service: &Service,
	replicas: usize,
) -> Result<()> {
	if replicas <= 1 {
		return Ok(());
	}
	let fixed: Vec<u16> = crate::ports::parse_ports(&service.ports)?
		.iter()
		.filter_map(|p| p.host_port)
		.filter(|&hp| hp != 0)
		.collect();
	if fixed.is_empty() {
		return Ok(());
	}
	Err(ComposeError::ScalePortConflict {
		service: service_name.to_string(),
		replicas,
		ports: fixed,
	})
}

impl Engine {
	/// Set the number of running containers for the named services (docker
	/// `compose scale SERVICE=N`). Creates missing replicas and removes any
	/// surplus. The `--scale` overrides are already applied to this engine, so
	/// `resolve_replicas` reports the target count during the up pass.
	pub async fn scale(&self, file: &ComposeFile, pairs: &[(String, u32)]) -> Result<()> {
		for (svc, _) in pairs {
			if !file.services.contains_key(svc) {
				return Err(ComposeError::ServiceNotFound(svc.clone()));
			}
		}
		// Fail fast on an over-limit count or a fixed host port before touching
		// any container.
		for (svc, target) in pairs {
			check_replica_limit(svc, *target as usize)?;
			check_scale_port_conflict(svc, &file.services[svc], *target as usize)?;
		}
		// Scale up: create only the missing replicas of the named services
		// (no_recreate keeps existing ones; no_deps leaves dependencies alone).
		let targets: Vec<String> = pairs.iter().map(|(s, _)| s.clone()).collect();
		self.up_with_options(file, true, &[], &targets, true, false, true)
			.await?;
		// Scale down: remove replicas beyond the target count.
		for (svc, target) in pairs {
			self.remove_surplus_replicas(svc, &file.services[svc], *target)
				.await?;
		}
		Ok(())
	}

	/// Remove the containers of `service_name` whose names fall outside the
	/// desired `target`-replica set (the scale-down half of reconciliation).
	async fn remove_surplus_replicas(
		&self,
		service_name: &str,
		service: &Service,
		target: u32,
	) -> Result<()> {
		let base = self.container_name(service_name, service);
		let desired: HashSet<String> = if target <= 1 {
			std::iter::once(base).collect()
		} else {
			(1..=target).map(|i| format!("{base}-{i}")).collect()
		};
		let grace = self.grace_period_secs(service);
		for name in self
			.list_project_container_names(Some(service_name))
			.await?
		{
			if !desired.contains(&name) {
				// Scaling down removes surplus replicas but keeps their data
				// volumes (only `down -v` reclaims volumes).
				self.stop_and_remove(&name, grace, false).await;
			}
		}
		Ok(())
	}

	/// Stop (best-effort) then force-remove a container by name. With
	/// `remove_volumes`, the container's anonymous volumes are reclaimed too
	/// (`podman rm -v`), so a label-based teardown sweep does not leave image
	/// `VOLUME`/anonymous volumes behind.
	pub(super) async fn stop_and_remove(&self, name: &str, grace: i32, remove_volumes: bool) {
		let stop_path = format!(
			"{API_PREFIX}/containers/{}/stop?t={grace}",
			urlencoded(name)
		);
		let _ = self.client.post_empty_ok(&stop_path).await;
		let rm_path = super::container_rm_path(name, remove_volumes);
		if let Err(e) = self.client.delete_ok(&rm_path).await {
			tracing::debug!("scale-down rm {name}: {e}");
		} else {
			info!("removed {name}");
		}
	}

	/// All container names carrying this project's label, optionally narrowed to
	/// one service via the `podup.service` label. Lets reconciliation find
	/// scaled replicas that the compose file's default count no longer names.
	pub(crate) async fn list_project_container_names(
		&self,
		service: Option<&str>,
	) -> Result<Vec<String>> {
		let mut labels = vec![format!("podup.project={}", self.project)];
		if let Some(svc) = service {
			labels.push(format!("podup.service={svc}"));
		}
		let filters = serde_json::json!({ "label": labels });
		let path = format!(
			"{API_PREFIX}/containers/json?all=true&filters={}",
			urlencoded(&filters.to_string()),
		);
		let entries = self
			.client
			.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;
		Ok(entries
			.into_iter()
			.flat_map(|e| e.names)
			.map(|raw| raw.trim_start_matches('/').to_string())
			.collect())
	}

	/// All live project containers grouped by their `podup.service` label, in a
	/// single API call. Lets a whole-project command (e.g. `down`) avoid one
	/// per-service container-list round-trip; callers fall back to the static
	/// [`Engine::replica_names`] for a service absent from the map.
	pub(crate) async fn list_project_containers_by_service(
		&self,
	) -> Result<std::collections::HashMap<String, Vec<String>>> {
		let filters = serde_json::json!({ "label": [format!("podup.project={}", self.project)] });
		let path = format!(
			"{API_PREFIX}/containers/json?all=true&filters={}",
			urlencoded(&filters.to_string()),
		);
		let entries = self
			.client
			.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;
		let mut by_service: std::collections::HashMap<String, Vec<String>> =
			std::collections::HashMap::new();
		for entry in entries {
			let Some(service) = entry.labels.get("podup.service") else {
				continue;
			};
			if let Some(raw) = entry.names.first() {
				by_service
					.entry(service.clone())
					.or_default()
					.push(raw.trim_start_matches('/').to_string());
			}
		}
		Ok(by_service)
	}

	/// The container names to act on for a service: the ones Podman actually has
	/// (matched by the `podup.service` label), so lifecycle and query commands
	/// keep working after a runtime `scale`/`up --scale` that the compose file's
	/// static replica count no longer names. Falls back to the statically-derived
	/// names when none exist yet (e.g. a service not yet created).
	pub(crate) async fn live_replica_names(
		&self,
		service_name: &str,
		service: &Service,
	) -> Result<Vec<String>> {
		let live = self
			.list_project_container_names(Some(service_name))
			.await?;
		Ok(if live.is_empty() {
			self.replica_names(service_name, service)
		} else {
			live
		})
	}
}

#[cfg(test)]
mod tests {
	use super::{check_replica_limit, check_scale_port_conflict, DEFAULT_MAX_REPLICAS};

	#[test]
	fn replica_limit_default_and_env_override() {
		// One test owns the shared `PODUP_MAX_REPLICAS` env var for its whole body
		// so a sibling test running in parallel can never race it.
		let max = DEFAULT_MAX_REPLICAS as usize;

		// Default ceiling: at-limit allowed, over-limit rejected.
		std::env::remove_var("PODUP_MAX_REPLICAS");
		assert!(check_replica_limit("web", 1).is_ok());
		assert!(check_replica_limit("web", max).is_ok());
		let err = check_replica_limit("web", max + 1).unwrap_err();
		assert!(matches!(
			err,
			crate::error::ComposeError::ReplicaLimitExceeded { .. }
		));
		assert!(check_replica_limit("web", 100_000).is_err());

		// Env override lowers the ceiling.
		std::env::set_var("PODUP_MAX_REPLICAS", "2");
		assert!(check_replica_limit("web", 2).is_ok());
		assert!(check_replica_limit("web", 3).is_err());

		// A zero/garbage override falls back to the default ceiling.
		std::env::set_var("PODUP_MAX_REPLICAS", "0");
		assert!(check_replica_limit("web", max).is_ok());
		std::env::set_var("PODUP_MAX_REPLICAS", "nope");
		assert!(check_replica_limit("web", max).is_ok());
		std::env::remove_var("PODUP_MAX_REPLICAS");
	}

	fn service(yaml: &str) -> crate::compose::types::Service {
		let file = crate::parse_str(yaml).unwrap();
		file.services.into_iter().next().unwrap().1
	}

	#[test]
	fn single_replica_never_conflicts() {
		let svc = service("services:\n  web:\n    image: x\n    ports:\n      - \"8080:80\"\n");
		assert!(check_scale_port_conflict("web", &svc, 1).is_ok());
	}

	#[test]
	fn scaled_fixed_host_port_conflicts() {
		let svc = service("services:\n  web:\n    image: x\n    ports:\n      - \"8080:80\"\n");
		let err = check_scale_port_conflict("web", &svc, 3).unwrap_err();
		assert!(matches!(
			err,
			crate::error::ComposeError::ScalePortConflict { .. }
		));
		assert!(err.to_string().contains("8080"));
	}

	#[test]
	fn scaled_random_host_port_is_allowed() {
		// A container-only port (`"80"`) gets a runtime-assigned host port per
		// replica, so scaling is fine.
		let svc = service("services:\n  web:\n    image: x\n    ports:\n      - \"80\"\n");
		assert!(check_scale_port_conflict("web", &svc, 3).is_ok());
	}

	#[test]
	fn scaled_no_ports_is_allowed() {
		let svc = service("services:\n  worker:\n    image: x\n");
		assert!(check_scale_port_conflict("worker", &svc, 5).is_ok());
	}
}
