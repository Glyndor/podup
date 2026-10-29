use super::super::note_if_idle;
use crate::compose::types::ComposeFile;
use crate::engine::lifecycle::drop_recheck::LifecycleGoal;
use crate::engine::lifecycle::parallel::{filter_levels, first_error, join_bounded};
use crate::engine::lifecycle::targets::{stop_deadline, stop_timeout_param};
use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::API_PREFIX;

impl Engine {
	/// Stop one container, escalating to an explicit `SIGKILL` if the libpod
	/// `stop` call does not complete within the grace window.
	///
	/// libpod normally `SIGKILL`s a container itself once the grace period lapses,
	/// so a healthy stop returns inside [`stop_deadline`]. If the call instead
	/// stalls (a daemon that accepts the request then never replies, or a
	/// container the server fails to reap), the bounded wait surfaces a timeout
	/// and we send `kill?signal=SIGKILL` so podup never depends solely on the
	/// server honouring `?t`. 304/404 are idempotent no-ops, as in
	/// [`run_lifecycle_op`](Self::run_lifecycle_op).
	///
	/// Returns `Ok(true)` when the container actually transitioned (or its
	/// state was re-confirmed via the drop-recheck path) and `Ok(false)` when
	/// libpod said it was already stopped/gone. The bool lets
	/// [`Self::stop_one_service`] keep its `acted` flag accurate now that it no
	/// longer filters by per-container state itself; the bulk container-list
	/// helper returns names without states, so the transition bit has to come
	/// from the response (#1363).
	pub(in crate::backends::engine::lifecycle) async fn stop_container(
		&self,
		container: &str,
		grace: i32,
	) -> Result<bool> {
		let path = format!(
			"{API_PREFIX}/containers/{}/stop?timeout={}",
			crate::libpod::urlencoded(container),
			stop_timeout_param(grace),
		);
		crate::ui::progress::start("Container", container, "Stopping");
		match self
			.client
			.post_empty_ok_within(&path, stop_deadline(grace))
			.await
		{
			Ok(()) => {
				crate::ui::progress_line("Container", container, "Stopped");
				Ok(true)
			}
			Err(e) if e.is_status(304) || e.is_status(404) => {
				tracing::debug!("{container}: stop skipped ({e})");
				crate::ui::progress_line("Container", container, "Skipped");
				Ok(false)
			}
			// `stop` is one of the four state-changing calls the drops were
			// measured on (#1339), and it does not go through `run_lifecycle_op`,
			// so it needs the re-check on its own.
			Err(e) if e.is_incomplete_message() => {
				self.confirm_lost_response(container, "Stopped", LifecycleGoal::NotRunning, e)
					.await
			}
			Err(e) if e.is_timeout() => {
				tracing::warn!("{container}: stop did not complete within the grace window; escalating to SIGKILL"
				);
				let kill_path = format!(
					"{API_PREFIX}/containers/{}/kill?signal=SIGKILL",
					crate::libpod::urlencoded(container),
				);
				match self.client.post_empty_ok(&kill_path).await {
					Ok(()) => {
						crate::ui::progress_line(
							"Container",
							container,
							"Killed (after stop timeout)",
						);
						Ok(true)
					}
					// Already gone / not running between the timeout and the kill.
					Err(e) if e.is_status(404) || e.is_status(409) => {
						tracing::debug!("{container}: SIGKILL skipped ({e})");
						Ok(false)
					}
					Err(e) => Err(ComposeError::Podman(e)),
				}
			}
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}

	/// Stop running containers without removing them.
	///
	/// Services are stopped in reverse dependency order. If `target_services`
	/// is empty, all services in the compose file are stopped.
	pub async fn stop(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		// Prefetch every project container once and group by service, instead of
		// one container-list round-trip per service (S+1 → 1 for the level walk;
		// #1363). The bulk helper returns every project's containers regardless
		// of state (with `all=true`); the per-container `acted` flag still has to
		// come from the `stop` response itself now that we no longer filter
		// running/paused client-side.
		let live_by_service = self.live_project_replicas().await?;
		self.stop_replicas(file, target_services, live_by_service)
			.await
	}

	/// Note which containers the project holds now, by name and ID, so a later
	/// [`Engine::stop_snapshot`] can stop these and no others.
	pub async fn snapshot_containers(&self) -> Result<ContainerSnapshot> {
		let mut by_service: std::collections::HashMap<String, Vec<(String, String)>> =
			std::collections::HashMap::new();
		for entry in self.project_container_entries().await? {
			let (Some(service), Some(name)) =
				(entry.labels.get("podup.service"), entry.names.first())
			else {
				continue;
			};
			by_service
				.entry(service.clone())
				.or_default()
				.push((name.trim_start_matches('/').to_string(), entry.id));
		}
		Ok(ContainerSnapshot { by_service })
	}

	/// Stop the containers of a [`ContainerSnapshot`] that are still the same
	/// containers: a name that now belongs to a container with another ID was
	/// replaced by someone else since the snapshot, and is left running.
	pub async fn stop_snapshot(
		&self,
		file: &ComposeFile,
		snapshot: &ContainerSnapshot,
	) -> Result<()> {
		let current: std::collections::HashMap<String, String> = self
			.project_container_entries()
			.await?
			.into_iter()
			.filter_map(|entry| {
				let name = entry.names.first()?.trim_start_matches('/').to_string();
				Some((name, entry.id))
			})
			.collect();
		let live_by_service = snapshot
			.by_service
			.iter()
			.map(|(service, containers)| {
				let same: Vec<String> = containers
					.iter()
					.filter(|(name, id)| current.get(name) == Some(id))
					.map(|(name, _)| name.clone())
					.collect();
				(service.clone(), same)
			})
			.collect();
		self.stop_replicas(file, &[], live_by_service).await
	}

	async fn stop_replicas(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		live_by_service: std::collections::HashMap<String, Vec<String>>,
	) -> Result<()> {
		// Stop in reverse dependency order (dependents before their dependencies),
		// one level at a time, but stop every service within a level concurrently
		// so independent grace periods overlap instead of summing (#757).
		let mut levels = crate::compose::resolve_levels(file)?;
		levels.reverse();
		let levels = filter_levels(file, levels, target_services)?;

		// Report "stopped" solely for containers actually running/paused:
		// stopping a Created/Exited one is a harmless no-op and must not claim
		// it stopped (#876), matching docker compose. `stop_container` returns
		// `Ok(false)` for libpod's 304/404 idempotent no-ops, which is how this
		// flag stays accurate with the bulk listing.
		let acted = std::sync::atomic::AtomicBool::new(false);
		// The board over the containers this stop will walk, seeded in the
		// reversed level order the walk below uses.
		crate::ui::progress::begin(crate::engine::lifecycle::seed::level_container_resources(
			&levels,
			&live_by_service,
		));
		let outcome = async {
			for level in &levels {
				let futs = level.iter().map(|name| {
					let grace = self.grace_period_secs(&file.services[name]);
					let names = live_by_service.get(name).cloned().unwrap_or_default();
					self.stop_one_service(names, grace, &acted)
				});
				if let Some(e) = first_error(join_bounded(futs).await) {
					return Err(e);
				}
			}
			Ok(())
		}
		.await;
		crate::ui::progress::end();
		outcome?;
		note_if_idle(&acted, "stop");
		Ok(())
	}
}

/// The containers a project held at some moment, by service, each as its name
/// and its ID. The ID is what tells a container from a later one that took the
/// same name.
#[derive(Debug, Clone, Default)]
pub struct ContainerSnapshot {
	by_service: std::collections::HashMap<String, Vec<(String, String)>>,
}
