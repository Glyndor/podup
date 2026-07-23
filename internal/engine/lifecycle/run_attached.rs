//! Interactive `run`: a pseudo-terminal on a one-shot container.
//!
//! `podup run -it app bash` parsed `-i` and `-T` and acted on neither: the
//! container got no TTY and no live stdin, so the flags described something that
//! did not happen (#1140, the half of #1079 that did not ship).
//!
//! The order here is the whole point. `attach` opens before `start`, because a
//! container that has already run has already printed, and for a one-shot `run`
//! that missed output is often all the output there was. `exec` does not have
//! this problem — the container is already up — which is why it can attach and
//! start in a single call and this cannot.

use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

impl super::super::Engine {
	/// Attach to a created-but-not-started container, start it, and hand the
	/// terminal over until the command exits. Returns its exit code.
	pub(super) async fn run_attached(&self, container_name: &str) -> Result<i64> {
		// stdin=1 so keystrokes reach the command; stream=1 keeps the connection
		// open both ways. With a TTY the stream is raw — no 8-byte frame headers,
		// because the pty merges stdout and stderr — which is also why an
		// interactive run cannot separate them, exactly as `podman run -it`.
		let attach_path = format!(
			"{API_PREFIX}/containers/{}/attach?stream=1&stdin=1&stdout=1&stderr=1",
			urlencoded(container_name),
		);
		let hijacked = self
			.client
			.post_hijack(&attach_path, b"")
			.await
			.map_err(ComposeError::Podman)?;

		let start_path = format!(
			"{API_PREFIX}/containers/{}/start",
			urlencoded(container_name),
		);
		self.client
			.post_empty_ok(&start_path)
			.await
			.map_err(ComposeError::Podman)?;

		// Raw mode only once the container is known to have started; entering it
		// before would leave the caller's terminal unusable behind a failed start.
		self.pump_terminal(
			hijacked,
			&format!("containers/{}", urlencoded(container_name)),
		)
		.await?;

		// The stream ending means the command finished; ask for its status.
		let wait_path = format!(
			"{API_PREFIX}/containers/{}/wait?condition=stopped",
			urlencoded(container_name),
		);
		self.client
			.post_empty_json_unbounded::<i64>(&wait_path)
			.await
			.map_err(ComposeError::Podman)
	}
}

impl super::super::Engine {
	/// Attach, start, hand over the terminal, then clean up per `--rm` and map
	/// the exit code. Split from `run` so the interactive tail reads as one
	/// piece.
	pub(super) async fn finish_interactive_run(
		&self,
		run_name: &str,
		rm: bool,
		rm_path: &str,
	) -> Result<()> {
		let outcome = self.run_attached(run_name).await;
		if rm {
			let _ = self.client.delete_ok(rm_path).await;
		}
		match outcome? {
			0 => Ok(()),
			code => Err(ComposeError::RunExited(code)),
		}
	}
}
