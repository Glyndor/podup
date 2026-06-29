//! Lifecycle sub-commands: restart, stop, start, kill, rm, pause, unpause, run.

use std::collections::HashSet;

use tracing::info;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

use super::filter_services;
use super::targets::{stop_deadline, stop_timeout_param};
use crate::engine::Engine;
use crate::libpod::API_PREFIX;

impl Engine {
	/// Run a lifecycle POST against one container with a consistent outcome:
	/// success logs `{done} {container}`; "already in the desired state" (304)
	/// and "no such container" (404) are idempotent no-ops; any other failure is
	/// a real error that propagates (setting a non-zero exit) instead of being
	/// swallowed into a warning. Shared by stop/start/restart/kill/pause/unpause
	/// so they all behave the same.
	async fn run_lifecycle_op(&self, path: &str, container: &str, done: &str) -> Result<()> {
		match self.client.post_empty_ok(path).await {
			Ok(()) => {
				info!("{done} {container}");
				Ok(())
			}
			Err(e) if e.is_status(304) || e.is_status(404) || e.is_kill_of_stopped() => {
				tracing::debug!("{container}: {done} skipped ({e})");
				Ok(())
			}
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}

	/// Like [`Self::run_lifecycle_op`] but also treats a "container state
	/// improper" error (already paused / not paused / not running) as an
	/// idempotent no-op. Podman rejects `pause`/`unpause` with a 409/500 when the
	/// container is not in the expected state; docker compose treats those as
	/// no-ops, so re-pausing or unpausing a not-paused container is harmless.
	async fn run_idempotent_state_op(&self, path: &str, container: &str, done: &str) -> Result<()> {
		match self.client.post_empty_ok(path).await {
			Ok(()) => {
				info!("{done} {container}");
				Ok(())
			}
			Err(e) if e.is_status(304) || e.is_status(404) || e.is_state_conflict() => {
				tracing::debug!("{container}: {done} skipped ({e})");
				Ok(())
			}
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}

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
	async fn stop_container(&self, container: &str, grace: i32) -> Result<()> {
		let path = format!(
			"{API_PREFIX}/containers/{}/stop?t={}",
			crate::libpod::urlencoded(container),
			stop_timeout_param(grace),
		);
		match self
			.client
			.post_empty_ok_within(&path, stop_deadline(grace))
			.await
		{
			Ok(()) => {
				info!("stopped {container}");
				Ok(())
			}
			Err(e) if e.is_status(304) || e.is_status(404) => {
				tracing::debug!("{container}: stop skipped ({e})");
				Ok(())
			}
			Err(e) if e.is_timeout() => {
				tracing::warn!(
					"{container}: stop did not complete within the grace window; escalating to SIGKILL"
				);
				let kill_path = format!(
					"{API_PREFIX}/containers/{}/kill?signal=SIGKILL",
					crate::libpod::urlencoded(container),
				);
				match self.client.post_empty_ok(&kill_path).await {
					Ok(()) => {
						info!("killed {container} (SIGKILL after stop timeout)");
						Ok(())
					}
					// Already gone / not running between the timeout and the kill.
					Err(e) if e.is_status(404) || e.is_status(409) => {
						tracing::debug!("{container}: SIGKILL skipped ({e})");
						Ok(())
					}
					Err(e) => Err(ComposeError::Podman(e)),
				}
			}
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}

	/// Restart the named service (or all services). Dependents with a `restart` condition in `depends_on` are also restarted.
	pub async fn restart(&self, file: &ComposeFile, service_name: Option<&str>) -> Result<()> {
		let targets: Vec<String> = service_name
			.map(|s| vec![s.to_string()])
			.unwrap_or_default();
		self.restart_with_options(file, &targets, false).await
	}

	/// Restart with options. When `target_services` is empty, all services are
	/// restarted. When `no_deps` is true, dependents with a `depends_on` restart
	/// condition are NOT cascade-restarted.
	pub async fn restart_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		no_deps: bool,
	) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let names = filter_services(file, order, target_services)?;

		// Containers already restarted, so a service that is both an explicit
		// target and a restart-dependent of another target is restarted once.
		let mut restarted: HashSet<String> = HashSet::new();
		// Attempt every container and surface the first error at the end rather
		// than aborting mid-batch and leaving later replicas/services unrestarted.
		let mut first_err: Option<ComposeError> = None;

		for name in &names {
			let service = &file.services[name];

			for container_name in self.live_replica_names(name, service).await? {
				if !restarted.insert(container_name.clone()) {
					continue;
				}
				let grace = self.grace_period_secs(service);
				// Single atomic restart (no visible stopped window) instead of a
				// stop+start round-trip.
				let restart_path = format!(
					"{API_PREFIX}/containers/{}/restart?t={}",
					crate::libpod::urlencoded(&container_name),
					stop_timeout_param(grace),
				);
				if let Err(e) = self
					.run_lifecycle_op(&restart_path, &container_name, "restarted")
					.await
				{
					first_err.get_or_insert(e);
				}
			}

			if no_deps {
				continue;
			}
			for (dep_name, dep_service) in &file.services {
				if dep_service.depends_on.restart_for(name) {
					for dep_container in self.live_replica_names(dep_name, dep_service).await? {
						if !restarted.insert(dep_container.clone()) {
							continue;
						}
						let grace = self.grace_period_secs(dep_service);
						let restart_path = format!(
							"{API_PREFIX}/containers/{}/restart?t={}",
							crate::libpod::urlencoded(&dep_container),
							stop_timeout_param(grace),
						);
						// Same 304/404 idempotency as the main path: a never-created
						// dependency must not spew a spurious cascade warning.
						if let Err(e) = self
							.run_lifecycle_op(&restart_path, &dep_container, "cascade-restarted")
							.await
						{
							first_err.get_or_insert(e);
						}
					}
				}
			}
		}

		if let Some(e) = first_err {
			return Err(e);
		}
		Ok(())
	}

	/// Block until each targeted service's containers stop, printing each exit
	/// code (`docker compose wait`). Returns `RunExited` with the last non-zero
	/// code so the process exit status reflects it, mirroring docker compose.
	pub async fn wait_services(
		&self,
		file: &ComposeFile,
		target_services: &[String],
	) -> Result<()> {
		// `docker compose wait` prints each service's exit code in the order the
		// services were given on the command line (deduplicated). Only fall back to
		// dependency order when no services were named (the "all" case).
		let order = if target_services.is_empty() {
			let order = crate::compose::resolve_order(file)?;
			filter_services(file, order, &[])?
		} else {
			for name in target_services {
				if !file.services.contains_key(name) {
					return Err(ComposeError::ServiceNotFound(name.clone()));
				}
			}
			let mut seen = std::collections::HashSet::new();
			target_services
				.iter()
				.filter(|n| seen.insert(n.as_str()))
				.cloned()
				.collect::<Vec<_>>()
		};

		let mut last_nonzero = 0i64;
		for name in &order {
			let service = &file.services[name];
			for container_name in self.live_replica_names(name, service).await? {
				let path = format!(
					"{API_PREFIX}/containers/{}/wait?condition=stopped",
					crate::libpod::urlencoded(&container_name),
				);
				let code = self
					.client
					.post_empty_json_unbounded::<i64>(&path)
					.await
					.map_err(ComposeError::Podman)?;
				println!("{code}");
				if code != 0 {
					last_nonzero = code;
				}
			}
		}
		if last_nonzero != 0 {
			return Err(ComposeError::RunExited(last_nonzero));
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
			for container_name in self.live_replica_names(name, service).await? {
				let grace = self.grace_period_secs(service);
				self.stop_container(&container_name, grace).await?;
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

		// Only act on containers Podman actually has. Acting on the static
		// fallback names (`live_replica_names`) would POST `/start` to containers
		// that were never created, 404 (swallowed as a no-op), and exit 0
		// silently — masking that the project was never created. Attempt every
		// live container and aggregate errors rather than aborting on the first.
		let mut any_live = false;
		let mut first_err: Option<ComposeError> = None;
		for name in &order {
			let live = self
				.list_project_container_names(Some(name.as_str()))
				.await?;
			if live.is_empty() {
				continue;
			}
			any_live = true;
			for container_name in live {
				let path = format!(
					"{API_PREFIX}/containers/{}/start",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self
					.run_lifecycle_op(&path, &container_name, "started")
					.await
				{
					first_err.get_or_insert(e);
				}
			}
		}

		if let Some(e) = first_err {
			return Err(e);
		}
		if !any_live {
			eprintln!("podup: no containers to start (project not created)");
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
		// Reject an empty/whitespace-only or otherwise invalid signal before
		// issuing any request — libpod would silently treat `signal=` as SIGKILL.
		super::signal::validate_signal(signal)?;

		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		for name in &order {
			let service = &file.services[name];
			for container_name in self.live_replica_names(name, service).await? {
				let path = format!(
					"{API_PREFIX}/containers/{}/kill?signal={}",
					crate::libpod::urlencoded(&container_name),
					crate::libpod::urlencoded(signal),
				);
				self.run_lifecycle_op(&path, &container_name, &format!("sent {signal} to"))
					.await?;
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
		self.rm_with_options(file, target_services, force, false)
			.await
	}

	/// Remove stopped service containers. `remove_volumes` (`-v/--volumes`) also
	/// removes anonymous volumes attached to each container.
	pub async fn rm_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		force: bool,
		remove_volumes: bool,
	) -> Result<()> {
		let mut order = crate::compose::resolve_order(file)?;
		order.reverse();
		let order = filter_services(file, order, target_services)?;

		let mut first_err: Option<ComposeError> = None;
		for name in &order {
			let service = &file.services[name];
			for container_name in self.live_replica_names(name, service).await? {
				let force_str = if force { "true" } else { "false" };
				let path = format!(
					"{API_PREFIX}/containers/{}?force={force_str}&v={remove_volumes}",
					crate::libpod::urlencoded(&container_name),
				);
				match self.client.delete_existed(&path).await {
					// Only report a removal that actually happened — a phantom
					// (never-created) container 404s and must not be logged as
					// "removed".
					Ok(true) => info!("removed {container_name}"),
					Ok(false) => {}
					// Without `--force`, a running container 409s. docker compose rm
					// skips running containers rather than aborting the batch, so
					// warn and keep going (later stopped containers still get
					// removed). The "Remove stopped service containers" help text
					// already promises this.
					Err(e) if !force && e.is_status(409) => {
						tracing::warn!(
							"{container_name} is running — skipping (pass -f to force removal)"
						);
					}
					Err(e) => {
						first_err.get_or_insert(ComposeError::Podman(e));
					}
				}
			}
		}
		if let Some(e) = first_err {
			return Err(e);
		}
		Ok(())
	}

	/// Pause running service containers (SIGSTOP).
	///
	/// If `target_services` is empty, all services are paused.
	pub async fn pause(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		// Idempotent + best-effort: re-pausing an already-paused (or stopped)
		// container is a no-op, and one state-mismatched container must not abort
		// the batch and leave the rest in an inconsistent partial state.
		let mut first_err: Option<ComposeError> = None;
		for name in &order {
			let service = &file.services[name];
			for container_name in self.live_replica_names(name, service).await? {
				let path = format!(
					"{API_PREFIX}/containers/{}/pause",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self
					.run_idempotent_state_op(&path, &container_name, "paused")
					.await
				{
					first_err.get_or_insert(e);
				}
			}
		}
		if let Some(e) = first_err {
			return Err(e);
		}
		Ok(())
	}

	/// Resume paused service containers.
	///
	/// If `target_services` is empty, all services are unpaused.
	pub async fn unpause(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		let order = crate::compose::resolve_order(file)?;
		let order = filter_services(file, order, target_services)?;

		// Idempotent + best-effort, mirroring `pause`: unpausing a not-paused
		// container is a no-op, and a single mismatch must not abort the batch.
		let mut first_err: Option<ComposeError> = None;
		for name in &order {
			let service = &file.services[name];
			for container_name in self.live_replica_names(name, service).await? {
				let path = format!(
					"{API_PREFIX}/containers/{}/unpause",
					crate::libpod::urlencoded(&container_name),
				);
				if let Err(e) = self
					.run_idempotent_state_op(&path, &container_name, "unpaused")
					.await
				{
					first_err.get_or_insert(e);
				}
			}
		}
		if let Some(e) = first_err {
			return Err(e);
		}
		Ok(())
	}

	/// True when a container with this exact name exists (any project). Used to
	/// refuse clobbering a pre-existing container on `run --name`.
	pub(super) async fn container_exists(&self, name: &str) -> Result<bool> {
		let path = format!(
			"{API_PREFIX}/containers/{}/json",
			crate::libpod::urlencoded(name),
		);
		match self.client.get_json::<serde_json::Value>(&path).await {
			Ok(_) => Ok(true),
			Err(e) if e.is_status(404) => Ok(false),
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}
}
