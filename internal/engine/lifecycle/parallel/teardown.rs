//! Teardown of one already-known-live container, as a unit of work in a
//! concurrent teardown level or batch.

use crate::compose::types::LifecycleHook;
use crate::engine::lifecycle::drop_recheck::LifecycleGoal;
use crate::engine::lifecycle::targets::{stop_deadline, stop_timeout_param};
use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

impl Engine {
	/// Tear down one already-known-live container: run its `pre_stop` hooks (if
	/// any), a best-effort stop bounded by `grace`, then a forced removal. One
	/// unit of work in a concurrent teardown level/batch, shared by
	/// [`Engine::down_with_options`] (per dependency level) and
	/// [`Engine::down_by_label`] (one label-scoped batch, no dependency
	/// graph to level).
	///
	/// A stalled or failed `stop` is never surfaced as an error here: the
	/// forced removal that follows SIGKILLs the container regardless of how
	/// `stop` went (`container_rm_path` always passes `force=true`), so only a
	/// genuine removal failure propagates. A 404 (container already gone) is an
	/// idempotent no-op at every step. This preserves the pre-parallel `down`
	/// error semantics (#598) byte-for-byte; only the dispatch became
	/// concurrent.
	pub(in crate::engine::lifecycle) async fn teardown_one_container(
		&self,
		container_name: &str,
		grace: i32,
		pre_stop: &[LifecycleHook],
		remove_volumes: bool,
	) -> Result<()> {
		crate::ui::progress::start("Container", container_name, "Stopping");
		for hook in pre_stop {
			if let Err(e) = self.run_lifecycle_hook(container_name, hook).await {
				tracing::debug!("pre_stop hook {container_name}: {e}");
			}
		}

		// Bound the stop by the grace window so a container ignoring SIGTERM
		// does not pin recreation for the full client READ_TIMEOUT; the
		// force-remove below SIGKILLs it regardless.
		let stop_path = format!(
			"{API_PREFIX}/containers/{}/stop?timeout={}",
			urlencoded(container_name),
			stop_timeout_param(grace),
		);
		// A 404 (container already gone, or a profile-gated service that was
		// never created) is an idempotent no-op here, exactly as the network and
		// volume removal arms treat it, not a warning. A stalled or failed stop
		// is not fatal either: the force-remove just below SIGKILLs the
		// container regardless, so its outcome is logged but never returned as
		// an error; only a genuine removal failure is.
		if let Err(e) = self
			.client
			.post_empty_ok_within(&stop_path, stop_deadline(grace))
			.await
		{
			if !e.is_status(404) {
				tracing::warn!("could not stop {container_name}: {e}");
			}
		}

		let rm_path =
			crate::engine::lifecycle::container_rm_path(container_name, remove_volumes, true);
		match self.client.delete_ok(&rm_path).await {
			Ok(()) => {
				crate::ui::progress_line("Container", container_name, "Removed");
				Ok(())
			}
			// The container was already gone (404), so nothing to do, but the row
			// has to close, or the live board leaves it spinning on `Stopping`
			// forever (#1347).
			Err(e) if e.is_status(404) => {
				crate::ui::progress_line("Container", container_name, "Absent");
				Ok(())
			}
			// The other state-changing call the drops were measured on (#1339).
			// `Gone` rather than `NotRunning`: a stopped-but-present container
			// would satisfy the latter and read a failed removal as a success.
			Err(e) if e.is_incomplete_message() => self
				.confirm_lost_response(container_name, "Removed", LifecycleGoal::Gone, e)
				.await
				.map(|_| ()),
			Err(e) => {
				tracing::warn!("could not remove {container_name}: {e}");
				// A `down` whose container removal genuinely failed previously
				// hid the failure behind a spinner (#1347).
				crate::ui::progress_line("Container", container_name, "Failed");
				Err(ComposeError::Podman(e))
			}
		}
	}
}
