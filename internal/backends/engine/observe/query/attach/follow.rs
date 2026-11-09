use futures_util::StreamExt;

use super::phase::StreamEnd;
use super::printer::ChunkPrinter;
use super::Engine;
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

/// One line of attached output as it is printed: `{prefix} | {text}`, or the
/// text alone when the prefix is off.
pub(super) fn render_line(prefix: &str, no_log_prefix: bool, text: &str) -> String {
	if no_log_prefix {
		text.to_string()
	} else {
		format!("{prefix} | {text}")
	}
}

/// How one container's log stream is followed.
pub(super) struct Follow<'a> {
	pub(super) service: &'a str,
	pub(super) display: &'a str,
	pub(super) container: &'a str,
	pub(super) timestamps: bool,
	pub(super) no_log_prefix: bool,
	/// Look up the exit status once the container stops, so a clean exit is
	/// told apart from a failure (`--abort-on-container-failure`).
	pub(super) check_failure: bool,
}

impl Engine {
	/// Follow one container's logs until its stream ends, printing each line.
	/// Returns the service, the container and how the stream ended.
	pub(super) async fn follow_container(&self, follow: Follow<'_>) -> (String, String, StreamEnd) {
		let Follow {
			service,
			display,
			container,
			timestamps,
			no_log_prefix,
			check_failure,
		} = follow;
		let end = self
			.follow_stream(display, container, timestamps, no_log_prefix)
			.await;
		let end = match end {
			StreamEnd::ContainerStopped if check_failure => {
				// A status that cannot be read is reported as a failure: the
				// abort path asks again and surfaces the error, instead of a
				// broken container being taken for a clean one.
				match self.container_exit_code(service, container).await {
					Ok(0) => StreamEnd::ContainerStopped,
					_ => StreamEnd::Failed,
				}
			}
			other => other,
		};
		(service.to_string(), container.to_string(), end)
	}

	async fn follow_stream(
		&self,
		prefix: &str,
		cname: &str,
		timestamps: bool,
		no_log_prefix: bool,
	) -> StreamEnd {
		// The libpod `/logs` endpoint always frames its body with 8-byte
		// multiplexed headers (stdout/stderr channel byte + payload length +
		// payload), including for containers started with a TTY; the raw path
		// is reserved for hijacked attach/exec streams, so no per-service TTY
		// check here.
		let path = format!(
			"{API_PREFIX}/containers/{}/logs?stdout=true&stderr=true&follow=true&timestamps={timestamps}",
			urlencoded(cname),
		);
		let resp = match self.client.get_stream(&path).await {
			Ok(r) => r,
			Err(e) => {
				tracing::warn!("attach_logs {prefix}: {e}");
				return StreamEnd::Broke;
			}
		};
		let mut stream = crate::libpod::parse_multiplexed(resp.into_body());
		let mut printer = ChunkPrinter::new(prefix).plain(no_log_prefix);
		while let Some(msg) = stream.next().await {
			match msg {
				// A write that fails for a reason other than a closed pipe means
				// the output cannot be delivered: the stream ends here rather
				// than looping over a sink that will not take the bytes.
				Ok(LogOutput::StdOut { message }) => {
					if printer.stdout(&message).is_err() {
						tracing::warn!("attach {prefix}: cannot write the output");
						return StreamEnd::Broke;
					}
				}
				Ok(LogOutput::StdErr { message }) => {
					if printer.stderr(&message).is_err() {
						tracing::warn!("attach {prefix}: cannot write the output");
						return StreamEnd::Broke;
					}
				}
				// An attach stream lives as long as its container and ends
				// when the container stops, so a lost chunked terminator is
				// indistinguishable at the transport layer from a real
				// mid-stream break (#1104). The container answers what the
				// transport cannot: still running means live output was
				// truncated.
				//
				// This arm used to discard the error without even a warning,
				// so `up` in the foreground could lose its connection to the
				// engine and still exit 0.
				Err(e) => {
					let _ = printer.finish();
					let kind = e.stream_end_kind();
					let still_running = self.container_still_running(cname).await;
					if super::super::stream_broke_mid_output(still_running) {
						tracing::warn!(
							"attach {prefix}: stream ended while the container was \
							 still running [{kind}]: {e}"
						);
						return StreamEnd::Broke;
					}
					tracing::warn!(
						"attach {prefix}: stream ended as the container stopped [{kind}]"
					);
					return StreamEnd::ContainerStopped;
				}
			}
		}
		let _ = printer.finish();
		// Clean end of the stream: the container stopped and the transport
		// delivered its terminator.
		StreamEnd::ContainerStopped
	}
}

#[cfg(test)]
#[path = "follow_tests.rs"]
mod tests;
