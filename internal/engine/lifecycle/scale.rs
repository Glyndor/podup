//! The `scale` subcommand plus the replica-listing/reconciliation helpers
//! shared with teardown.

use std::collections::HashSet;

use tracing::info;

use crate::compose::types::{ComposeFile, Service};
use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::check_scale_port_conflict;

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
		// Fail fast on a fixed host port before touching any container.
		for (svc, target) in pairs {
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
				self.stop_and_remove(&name, grace).await;
			}
		}
		Ok(())
	}

	/// Stop (best-effort) then force-remove a container by name.
	pub(super) async fn stop_and_remove(&self, name: &str, grace: i32) {
		let stop_path = format!(
			"{API_PREFIX}/containers/{}/stop?t={grace}",
			urlencoded(name)
		);
		let _ = self.client.post_empty_ok(&stop_path).await;
		let rm_path = format!("{API_PREFIX}/containers/{}?force=true", urlencoded(name));
		if let Err(e) = self.client.delete_ok(&rm_path).await {
			tracing::debug!("scale-down rm {name}: {e}");
		} else {
			info!("removed {name}");
		}
	}

	/// All container names carrying this project's label, optionally narrowed to
	/// one service via the `podup.service` label. Lets reconciliation find
	/// scaled replicas that the compose file's default count no longer names.
	pub(super) async fn list_project_container_names(
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
}
