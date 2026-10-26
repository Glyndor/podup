//! Lifecycle sub-commands: restart, stop, start, kill, rm, pause, unpause, wait.

mod rm;
mod running;
mod signals;
mod wait;

pub(super) use signals::kill_signal;

use crate::engine::lifecycle::drop_recheck::LifecycleGoal;
use crate::engine::Engine;
use crate::error::{ComposeError, Result};

/// Say so when a command finished having done nothing.
///
/// `start` already did this (*"no containers to start (project not created)"*)
/// and the other six lifecycle commands did not: `rm`, `stop`, `restart`, `kill`,
/// `pause` and `unpause` all exited 0 in complete silence on a project that was
/// never created, which reads exactly like success. Measured before the fix, all
/// six printed zero lines.
///
/// One helper rather than a literal per command, so the wording cannot drift into
/// seven dialects of the same sentence.
pub(super) fn note_if_idle(acted: &std::sync::atomic::AtomicBool, verb: &str) {
	if !acted.load(std::sync::atomic::Ordering::Relaxed) {
		crate::ui::progress_note(&format!("no containers to {verb}"));
	}
}

/// The verb a row shows while the engine is still working, from the verb it
/// will show when done. The board keeps a spinner and a clock on the row
/// between the two; a plain sink prints only the final one.
pub(super) fn working_verb(done: &str) -> &'static str {
	match done {
		"Started" => "Starting",
		"Stopped" => "Stopping",
		"Restarted" => "Restarting",
		"Killed" => "Killing",
		"Paused" => "Pausing",
		"Unpaused" => "Unpausing",
		"Removed" => "Removing",
		_ => "Working",
	}
}

impl Engine {
	/// Run a lifecycle POST against one container with a consistent outcome:
	/// success prints a `Container {container}  {done}` progress line; "already
	/// in the desired state" (304)
	/// and "no such container" (404) are idempotent no-ops; any other failure is
	/// a real error that propagates (setting a non-zero exit) instead of being
	/// swallowed into a warning. Shared by stop/start/restart/kill/pause/unpause
	/// so they all behave the same.
	///
	/// Returns whether the container was actually acted on. This is the only
	/// place that knows: the per-replica query helpers (e.g. the bulk
	/// [`Engine::live_project_replicas_sorted`] lookup used by `exec`/`cp`/
	/// `port`/`logs`) fall back to the *static* compose names when nothing is
	/// running, so a command on a project that was never created still walks
	/// a full list of container names and 404s on every one of them. Six
	/// commands exited 0 in complete silence that way (#1248), and telling
	/// that apart from real work requires the answer here, not a count of
	/// names upstream.
	pub(super) async fn run_lifecycle_op(
		&self,
		path: &str,
		container: &str,
		done: &str,
		goal: LifecycleGoal,
	) -> Result<bool> {
		crate::ui::progress::start("Container", container, working_verb(done));
		match self.client.post_empty_ok(path).await {
			Ok(()) => {
				crate::ui::progress_line("Container", container, done);
				Ok(true)
			}
			Err(e) if e.is_status(304) || e.is_status(404) || e.is_kill_of_stopped() => {
				tracing::debug!("{container}: {done} skipped ({e})");
				crate::ui::progress_line("Container", container, "Skipped");
				Ok(false)
			}
			// The server closed before completing the response. That is not an
			// answer: the operation may have run to completion and lost only its
			// reply. Measured on Podman 6 under concurrency, where the drops land
			// on exactly these state-changing POSTs and follow a slow one: a
			// restart that burned its full stop grace, then a drop on the next
			// (#1339). It is not a client deadline (READ_TIMEOUT is 120s) and not
			// a pooled-connection race (there is no pool; every request gets a
			// fresh socket), so the transport genuinely cannot say.
			//
			// Resolve it the way `cp` and `stats` already do: ask the observable
			// the transport cannot see. If the container reached the state the
			// operation was for, it succeeded.
			Err(e) if e.is_incomplete_message() => {
				self.confirm_lost_response(container, done, goal, e).await
			}
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}

	/// Like [`Self::run_lifecycle_op`] but also treats a "container state
	/// improper" error (already paused / not paused / not running) as an
	/// idempotent no-op. Podman rejects `pause`/`unpause` with a 409/500 when the
	/// container is not in the expected state; docker compose treats those as
	/// no-ops, so re-pausing or unpausing a not-paused container is harmless.
	pub(super) async fn run_idempotent_state_op(
		&self,
		path: &str,
		container: &str,
		done: &str,
	) -> Result<bool> {
		crate::ui::progress::start("Container", container, working_verb(done));
		match self.client.post_empty_ok(path).await {
			Ok(()) => {
				crate::ui::progress_line("Container", container, done);
				Ok(true)
			}
			Err(e) if e.is_status(304) || e.is_status(404) || e.is_state_conflict() => {
				tracing::debug!("{container}: {done} skipped ({e})");
				crate::ui::progress_line("Container", container, "Skipped");
				Ok(false)
			}
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}
}

#[cfg(all(test, unix))]
mod board_tests;

#[cfg(test)]
mod wait_output_tests;
