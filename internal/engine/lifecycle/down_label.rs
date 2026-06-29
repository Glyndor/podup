//! Label-scoped teardown: `down` with no compose file present.
//!
//! [`Engine::down_with_options`] walks the parsed service graph, so it needs a
//! compose file. `docker compose -p NAME down` must also tear a *running*
//! project down purely from its `podup.project` label when that file is absent
//! (e.g. it was deleted, or you only have the project name). This module reaps
//! every labelled container, network, named volume and internal secret without
//! reading a single service definition.

use tracing::info;

use crate::compose::types::ComposeFile;
use crate::error::Result;
use crate::libpod::{urlencoded, API_PREFIX};

use super::Engine;

/// Shutdown grace (seconds) for the label-only teardown, used when the CLI
/// `-t/--timeout` override is unset. There is no service definition to read a
/// `stop_grace_period` from, so this falls back to docker compose's default.
const DEFAULT_STOP_GRACE_SECS: i32 = 10;

impl Engine {
	/// Tear a project down purely from its `podup.project` label, with no compose
	/// file: stop and force-remove every labelled container (and, with
	/// `remove_volumes`, their anonymous volumes), then sweep the project's
	/// networks, named volumes and internal secrets by label.
	///
	/// Mirrors `docker compose -p NAME down` against a running project whose
	/// compose file is absent. Unlike [`Engine::down_with_options`], which works
	/// through the parsed service graph, this enumerates everything by label, so
	/// it reaps all of the project's resources regardless of whether a file would
	/// parse. Best-effort throughout: a missing resource (404) is an idempotent
	/// no-op and an individual delete failure is logged, not fatal.
	pub async fn down_by_label(&self, remove_volumes: bool) -> Result<()> {
		let grace = self.stop_timeout.unwrap_or(DEFAULT_STOP_GRACE_SECS);
		for container_name in self.list_project_container_names(None).await? {
			let stop_path = format!(
				"{API_PREFIX}/containers/{}/stop?t={}",
				urlencoded(&container_name),
				super::targets::stop_timeout_param(grace),
			);
			// A 404 (already gone) is an idempotent no-op, like the network/volume
			// arms below; the force-remove that follows SIGKILLs a stubborn one.
			if let Err(e) = self
				.client
				.post_empty_ok_within(&stop_path, super::targets::stop_deadline(grace))
				.await
			{
				if !e.is_status(404) {
					tracing::warn!("could not stop {container_name}: {e}");
				}
			}

			let rm_path = super::container_rm_path(&container_name, remove_volumes);
			match self.client.delete_ok(&rm_path).await {
				Ok(()) => info!("removed {container_name}"),
				Err(e) if e.is_status(404) => {}
				Err(e) => tracing::warn!("could not remove {container_name}: {e}"),
			}
		}

		// Networks, named volumes and internal secrets are all reaped by label.
		self.remove_project_networks_by_label().await;
		if remove_volumes {
			self.remove_project_volumes_by_label().await;
		}
		// An empty file carries no declared secrets/configs, so this falls straight
		// through to the by-label orphan sweep that already backs `down`.
		self.remove_internal_secrets(&ComposeFile::default())
			.await?;
		Ok(())
	}

	/// Remove every network carrying this project's `podup.project` label — the
	/// implicit `<project>_default`, a network whose compose key changed, or any
	/// project network when no file is present. Only podup-labelled networks
	/// match, so a user's external network is never touched. Best-effort: a list
	/// failure leaves networks in place rather than aborting teardown.
	///
	/// Shared by [`Engine::down_with_options`] and [`Engine::down_by_label`].
	pub(super) async fn remove_project_networks_by_label(&self) {
		let net_filters =
			serde_json::json!({ "label": [format!("podup.project={}", self.project)] });
		let list_path = format!(
			"{API_PREFIX}/networks/json?filters={}",
			urlencoded(&net_filters.to_string()),
		);
		let Ok(nets) = self
			.client
			.get_json::<Vec<serde_json::Value>>(&list_path)
			.await
		else {
			return;
		};
		for net in nets {
			let Some(net_name) = net.get("name").and_then(|n| n.as_str()) else {
				continue;
			};
			let del = format!("{API_PREFIX}/networks/{}", urlencoded(net_name));
			match self.client.delete_ok(&del).await {
				Ok(_) => info!("removed network {net_name}"),
				Err(e) if e.is_status(404) => {}
				Err(e) => tracing::warn!("could not remove network {net_name}: {e}"),
			}
		}
	}

	/// Remove every named volume carrying this project's `podup.project` label.
	/// libpod's `/volumes/json` is fetched in full and filtered client-side by
	/// the label (mirroring the secret sweep) so only podup-created volumes are
	/// deleted — a user's hand-made or external volume is never touched.
	/// Best-effort: a list failure leaves volumes in place.
	pub(super) async fn remove_project_volumes_by_label(&self) {
		let path = format!("{API_PREFIX}/volumes/json");
		let Ok(vols) = self.client.get_json::<Vec<serde_json::Value>>(&path).await else {
			return;
		};
		for vol in vols {
			if !volume_owned_by(&vol, &self.project) {
				continue;
			}
			let Some(name) = vol.get("Name").and_then(|n| n.as_str()) else {
				continue;
			};
			let del = format!("{API_PREFIX}/volumes/{}", urlencoded(name));
			match self.client.delete_ok(&del).await {
				Ok(_) => info!("removed volume {name}"),
				Err(e) if e.is_status(404) => {}
				Err(e) => tracing::warn!("could not remove volume {name}: {e}"),
			}
		}
	}
}

/// Whether a `/volumes/json` entry carries the `podup.project=<project>` label,
/// i.e. it is a volume podup created for this project. Pure so the ownership
/// check is unit-tested without a live Podman socket.
fn volume_owned_by(vol: &serde_json::Value, project: &str) -> bool {
	vol.get("Labels")
		.and_then(|l| l.get("podup.project"))
		.and_then(|v| v.as_str())
		== Some(project)
}

#[cfg(test)]
mod tests {
	use super::volume_owned_by;

	#[test]
	fn volume_owned_by_matches_project_label() {
		let vol = serde_json::json!({
			"Name": "proj_data",
			"Labels": { "podup.project": "proj", "extra": "1" },
		});
		assert!(volume_owned_by(&vol, "proj"));
		// A different project's volume, or one podup never labelled, is not ours.
		assert!(!volume_owned_by(&vol, "other"));
		let unlabelled = serde_json::json!({ "Name": "loose", "Labels": {} });
		assert!(!volume_owned_by(&unlabelled, "proj"));
		let no_labels = serde_json::json!({ "Name": "loose" });
		assert!(!volume_owned_by(&no_labels, "proj"));
	}
}
