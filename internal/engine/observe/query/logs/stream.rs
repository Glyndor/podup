//! How a log stream is labelled, queried and ended, shared by
//! `Engine::logs_with_display` (and the small delegation stubs `Engine::logs` /
//! `Engine::logs_with_options` in `mod.rs`): the label prefix, the write-error
//! rule, the query string and the out-of-band "still running?" predicate with
//! the Engine method that backs it.

use crate::libpod::{urlencoded, API_PREFIX};

use super::super::options::LogsOptions;
use super::super::Engine;

/// The label `logs` tags a container's lines with: the container name minus the
/// project prefix, so `myproj-web-1` reads as `web-1`.
///
/// Attached `up` already strips it this way (`attach.rs`), so before this the
/// same container was tagged `myproj-web-1  | ` by one command and `web-1 | ` by
/// the other: two shapes for one thing, in one binary. docker compose prints
/// the short form.
pub(super) fn display_label(container_name: &str, project: &str) -> String {
	container_name
		.strip_prefix(&format!("{project}-"))
		.unwrap_or(container_name)
		.to_string()
}

/// Whether a failed write to the log sink should end the follow loop.
///
/// A `BrokenPipe` is the ordinary way a piped consumer signals it has read
/// enough: `logs -f | head`, `| grep -q`, `| less` and quit. It is a clean end
/// of output, not a failure, and the loop must stop: podup used to discard the
/// write result entirely and go on streaming into a dead pipe until the process
/// was killed. Any other io error is worth a warning before stopping, since it
/// means output is being lost for a reason the user cannot see.
pub(super) fn stop_on_write_error(container_name: &str, result: std::io::Result<()>) -> bool {
	match result {
		Ok(()) => false,
		Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => true,
		Err(e) => {
			tracing::warn!("logs {container_name}: cannot write output: {e}");
			true
		}
	}
}

/// Whether a log stream that ended with an error truncated live output.
///
/// `still_running` is the out-of-band re-check: `Some(true)` the container is
/// still up, `Some(false)` it has stopped or is gone, `None` the state could not
/// be read.
///
/// `None` counts as a break. It is tempting to read "could not tell" as a clean
/// end, and that is wrong in the exact case this matters: the severed connection
/// that ends the stream is usually the same one the re-check needs, so treating
/// the unknown as success turns every transport failure back into exit 0, which
/// is the bug, not the fix. Measured: with the permissive version, restarting the
/// libpod socket under an attached `logs -f` reported a still-running container
/// as stopped.
///
/// Pure so the rule is pinned without a live socket.
pub(in crate::engine) fn stream_broke_mid_output(still_running: Option<bool>) -> bool {
	!matches!(still_running, Some(false))
}

/// Build the libpod `containers/{}/logs` query string from the options.
pub(super) fn log_query(opts: &LogsOptions) -> String {
	let mut q = format!(
		"stdout=true&stderr=true&follow={}&timestamps={}",
		opts.follow, opts.timestamps
	);
	if let Some(tail) = &opts.tail {
		q.push_str(&format!("&tail={}", urlencoded(tail)));
	}
	if let Some(since) = &opts.since {
		q.push_str(&format!("&since={}", urlencoded(since)));
	}
	if let Some(until) = &opts.until {
		q.push_str(&format!("&until={}", urlencoded(until)));
	}
	q
}

impl Engine {
	/// Whether `container_name` is still running, for resolving how a log stream
	/// ended.
	///
	/// A `logs -f` stream lives as long as its container runs and ends when the
	/// container stops. libpod signals that end with a chunked terminator, and a
	/// lost terminator (a dropped connection, or a version that omits it)
	/// arrives as an `Err` that the transport layer cannot tell apart from a real
	/// mid-stream break (#1104). Re-checking the container out of band resolves
	/// it: still running means the stream truncated live output.
	///
	/// `Ok(None)` is "could not tell": an unreadable state is not confirmation
	/// the end was expected, so the caller keeps the original error rather than
	/// masking a possible failure. Mirrors the fail-closed rule `stats` uses.
	pub(in crate::engine) async fn container_still_running(
		&self,
		container_name: &str,
	) -> Option<bool> {
		let path = format!(
			"{API_PREFIX}/containers/json?all=true&filters={}",
			self.project_label_filter_encoded(),
		);
		let entries = self
			.client
			.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
			.await
			.ok()?;
		entries
			.iter()
			.find(|e| {
				e.names
					.iter()
					.any(|raw| raw.trim_start_matches('/') == container_name)
			})
			.map(|e| e.state == "running")
			// A container the listing no longer holds has been removed, which is a
			// stop by any other name; the stream had every reason to end.
			.or(Some(false))
	}
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
