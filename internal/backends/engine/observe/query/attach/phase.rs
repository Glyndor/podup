use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;

/// How one stream of `attach_logs_with_options` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamEnd {
	/// The stream's transport died while its container was still running: a
	/// truncated live read, not a finished one (#1104).
	Broke,
	/// The container is stopped (clean end or transport died *because* the
	/// container stopped). The abort path treats this as the trigger.
	ContainerStopped,
}

/// What the multi-stream wait returned before the outer code acts on it.
pub(super) enum Phase {
	/// SIGINT/SIGTERM arrived.
	Interrupted,
	/// Every stream finished. `saw_break` is true if any one of them ended
	/// with a truncated read, which makes the outcome `StreamBroke` rather
	/// than `StreamsEnded`.
	AllEnded { saw_break: bool },
	/// `abort_on_container_exit` is set and the named stream finished while
	/// its container was stopped, the first container to exit. The outer
	/// code stops the rest and reports the exit code.
	FirstExited {
		trigger_service: String,
		trigger_container: String,
	},
}

pub(super) async fn wait_for_phase(
	mut streams: FuturesUnordered<impl futures_util::Future<Output = (String, String, StreamEnd)>>,
	abort_on_container_exit: bool,
) -> Phase {
	let mut saw_break = false;
	#[cfg(unix)]
	let mut sigterm = {
		use tokio::signal::unix::{signal, SignalKind};
		signal(SignalKind::terminate()).expect("SIGTERM handler")
	};
	loop {
		#[cfg(unix)]
		{
			tokio::select! {
				biased;
				_ = tokio::signal::ctrl_c() => return Phase::Interrupted,
				_ = sigterm.recv() => return Phase::Interrupted,
				next = streams.next() => match next {
					None => return Phase::AllEnded { saw_break },
					Some((svc, cname, end)) => match end {
						StreamEnd::Broke => saw_break = true,
						StreamEnd::ContainerStopped if abort_on_container_exit => {
							return Phase::FirstExited {
								trigger_service: svc,
								trigger_container: cname,
							};
						}
						// Without `--abort-on-container-exit` a container stopping
						// mid-stream is not an event: we keep waiting for the
						// others, and the loop terminates with `AllEnded` when
						// they all finish on their own.
						StreamEnd::ContainerStopped => {}
					},
				},
			}
		}
		#[cfg(not(unix))]
		{
			match streams.next().await {
				None => return Phase::AllEnded { saw_break },
				Some((svc, cname, end)) => match end {
					StreamEnd::Broke => saw_break = true,
					StreamEnd::ContainerStopped if abort_on_container_exit => {
						return Phase::FirstExited {
							trigger_service: svc,
							trigger_container: cname,
						};
					}
					StreamEnd::ContainerStopped => {}
				},
			}
		}
	}
}
