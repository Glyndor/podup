//! The per-service units of work a lifecycle command fans out over a
//! dependency level. Each one acts on the live containers of a single service
//! and reports the first error in container order, so the rest of the level is
//! allowed to finish before the failure surfaces. The teardown path, which
//! works per container rather than per service, lives in [`super::teardown`].

use crate::engine::lifecycle::drop_recheck::LifecycleGoal;
use crate::engine::lifecycle::targets::stop_timeout_param;
use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

impl Engine {
	/// Stop a single service's live containers, as one unit of work in a
	/// concurrent level. See [`Engine::stop`]. Takes its container names from
	/// the bulk project listing (`live_project_replicas`); the per-container
	/// `acted` flag still has to come from [`Self::stop_container`] itself
	/// because the bulk helper returns names without states (#1363).
	pub(in crate::backends::engine::lifecycle) async fn stop_one_service(
		&self,
		container_names: Vec<String>,
		grace: i32,
		acted: &std::sync::atomic::AtomicBool,
	) -> Result<()> {
		let mut first_err: Option<ComposeError> = None;
		for container_name in container_names {
			match self.stop_container(&container_name, grace).await {
				Ok(true) => acted.store(true, std::sync::atomic::Ordering::Relaxed),
				Ok(false) => {
					tracing::debug!("{container_name}: not running, stop is a no-op");
				}
				Err(e) => {
					first_err.get_or_insert(e);
				}
			}
		}
		first_err.map_or(Ok(()), Err)
	}

	/// Start a single service's live containers, recording in `any_live` whether
	/// the service had any container to act on. See [`Engine::start`].
	pub(in crate::backends::engine::lifecycle) async fn start_one_service(
		&self,
		container_names: Vec<String>,
		any_live: &std::sync::atomic::AtomicBool,
	) -> Result<()> {
		if container_names.is_empty() {
			return Ok(());
		}
		any_live.store(true, std::sync::atomic::Ordering::Relaxed);
		let mut first_err: Option<ComposeError> = None;
		for container_name in container_names {
			let path = format!(
				"{API_PREFIX}/containers/{}/start",
				urlencoded(&container_name),
			);
			if let Err(e) = self
				.run_lifecycle_op(&path, &container_name, "Started", LifecycleGoal::Running)
				.await
			{
				first_err.get_or_insert(e);
			}
		}
		first_err.map_or(Ok(()), Err)
	}

	/// Restart a single service's live containers. `done` is the log verb
	/// (`restarted` for a direct target, `cascade-restarted` for a dependent).
	pub(in crate::backends::engine::lifecycle) async fn restart_one_service(
		&self,
		container_names: Vec<String>,
		grace: i32,
		done: &str,
		acted: &std::sync::atomic::AtomicBool,
	) -> Result<()> {
		let mut first_err: Option<ComposeError> = None;
		for container_name in container_names {
			// Single atomic restart (no visible stopped window) instead of a
			// stop+start round-trip.
			let restart_path = format!(
				"{API_PREFIX}/containers/{}/restart?timeout={}",
				urlencoded(&container_name),
				stop_timeout_param(grace),
			);
			match self
				.run_lifecycle_op(&restart_path, &container_name, done, LifecycleGoal::Running)
				.await
			{
				Ok(true) => acted.store(true, std::sync::atomic::Ordering::Relaxed),
				Ok(false) => {}
				Err(e) => {
					first_err.get_or_insert(e);
				}
			}
		}
		first_err.map_or(Ok(()), Err)
	}

	/// Send `signal` to a single service's live containers. See [`Engine::kill`].
	pub(in crate::backends::engine::lifecycle) async fn kill_one_service(
		&self,
		container_names: Vec<String>,
		signal: &str,
		acted: &std::sync::atomic::AtomicBool,
	) -> Result<()> {
		let mut first_err: Option<ComposeError> = None;
		// The Docker compat `/kill` handler blocks on the container when the
		// signal is SIGKILL/KILL/9 (or 0), and returns once the container has
		// exited or stopped; the libpod handler replies immediately. The
		// compensation is a follow-up `wait?condition=stopped` for those
		// signals only; SIGTERM etc. are answered promptly either way, and
		// skipping the wait is what keeps `kill -s SIGTERM <id>` from pinning
		// the caller behind every container the user happens to target.
		let wait_for_exit =
			crate::engine::lifecycle::commands::kill_signal::must_wait_after_kill(signal);
		for container_name in container_names {
			let path = format!(
				"{API_PREFIX}/containers/{}/kill?signal={}",
				urlencoded(&container_name),
				urlencoded(signal),
			);
			match self
				.run_lifecycle_op(&path, &container_name, "Killed", LifecycleGoal::NotRunning)
				.await
			{
				Ok(true) => {
					acted.store(true, std::sync::atomic::Ordering::Relaxed);
					if wait_for_exit {
						self.wait_after_kill(&container_name).await;
					}
				}
				Ok(false) => {}
				Err(e) => {
					first_err.get_or_insert(e);
				}
			}
		}
		first_err.map_or(Ok(()), Err)
	}

	/// Block until `container` is exited or stopped, the way the Docker
	/// compat `/kill` handler did for SIGKILL/0. Capped by [`READ_TIMEOUT`]
	/// so a stuck wait cannot pin the CLI forever; the kill itself
	/// already succeeded, so a timeout here is logged at warn and not
	/// promoted to an error.
	async fn wait_after_kill(&self, container: &str) {
		let path = format!(
			"{API_PREFIX}/containers/{}/wait?condition=stopped",
			urlencoded(container),
		);
		match self.client.post_empty_json_unbounded::<i64>(&path).await {
			Ok(_code) => {}
			Err(e) => {
				tracing::warn!(
					"kill {container}: wait for exited/stopped did not complete [{e}]; \
					 the container is left to settle on its own"
				);
			}
		}
	}

	/// Remove a single service's containers. See [`Engine::rm_with_options`].
	pub(in crate::backends::engine::lifecycle) async fn rm_one_service(
		&self,
		container_names: Vec<String>,
		force: bool,
		remove_volumes: bool,
		acted: &std::sync::atomic::AtomicBool,
	) -> Result<()> {
		let mut first_err: Option<ComposeError> = None;
		for container_name in container_names {
			let path =
				crate::engine::lifecycle::container_rm_path(&container_name, remove_volumes, force);
			// The row opens with its working verb so it carries a start time and
			// `Removed` comes with the elapsed time, the way `down` reports the
			// same removal (#1686). Every arm closes the row: one left at
			// `Removing` would be flushed as such by the plain sink.
			crate::ui::progress::start("Container", &container_name, "Removing");
			match self.client.delete_existed(&path).await {
				// Only report a removal that actually happened: a phantom
				// (never-created) container 404s and must not be logged as
				// "removed".
				Ok(true) => {
					acted.store(true, std::sync::atomic::Ordering::Relaxed);
					crate::ui::progress_line("Container", &container_name, "Removed");
				}
				Ok(false) => crate::ui::progress_line("Container", &container_name, "Absent"),
				// Without `--force`, a running container 409s. docker compose rm
				// skips running containers rather than aborting, so warn and keep
				// going (later stopped containers still get removed).
				Err(e) if !force && e.is_status(409) => {
					crate::ui::progress_line("Container", &container_name, "Skipped");
					tracing::warn!(
						"{container_name} is running; skipping (pass -f to force removal)"
					);
				}
				Err(e) => {
					crate::ui::progress_line("Container", &container_name, "Failed");
					first_err.get_or_insert(ComposeError::Podman(e));
				}
			}
		}
		first_err.map_or(Ok(()), Err)
	}

	/// Pause or unpause a single service's live containers, treating a state
	/// mismatch as an idempotent no-op. `endpoint` is `pause`/`unpause`.
	pub(in crate::backends::engine::lifecycle) async fn idempotent_state_service(
		&self,
		container_names: Vec<String>,
		endpoint: &str,
		done: &str,
		acted: &std::sync::atomic::AtomicBool,
	) -> Result<()> {
		let mut first_err: Option<ComposeError> = None;
		for container_name in container_names {
			let path = format!(
				"{API_PREFIX}/containers/{}/{endpoint}",
				urlencoded(&container_name),
			);
			match self
				.run_idempotent_state_op(&path, &container_name, done)
				.await
			{
				Ok(true) => acted.store(true, std::sync::atomic::Ordering::Relaxed),
				Ok(false) => {}
				Err(e) => {
					first_err.get_or_insert(e);
				}
			}
		}
		first_err.map_or(Ok(()), Err)
	}
}
