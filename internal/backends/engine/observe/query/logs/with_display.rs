//! `Engine::logs_with_display` and the three private helpers it delegates to:
//!
//! * `resolve_log_targets`: validate services, resolve against the bulk
//!   replica map, build the target list.
//! * `stream_concurrent_targets`: the `follow && len > 1` parallel branch.
//! * `stream_one_target`: the sequential branch (one container at a time;
//!   stdout locked for the whole stream).
//!
//! Each helper owns its own complete stream loop, so the move is not a
//! mid-loop cut; the orchestrator below keeps the same call order as the
//! pre-split function.

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

use super::super::options::{LogsDisplay, LogsOptions};
use super::super::Engine;
use super::filters::validate_log_filters;
use super::prefix::LinePrefixer;
use super::stream::{display_label, log_query, stop_on_write_error, stream_lost_output};

/// Outcome of streaming one container on the sequential path.
enum StreamOneOutcome {
	/// Open failed (the helper has already recorded `streamed_err`); the
	/// parent should NOT set `streamed_any`, matching the pre-split
	/// `continue` that skipped every per-container side-effect.
	Failed,
	/// Open succeeded and the helper completed (any inner-loop end, with or
	/// without truncation: `truncated_err` is already recorded when it
	/// applies). The parent sets `streamed_any = true`.
	Streamed,
}

/// Outcome of streaming one container on the concurrent path.
///
/// The two failure classes have to stay apart, because the parent treats them
/// differently: a container that would not open a stream is tolerated while
/// a sibling streams, and a stream that died while its container kept
/// running means output was lost, which no sibling can excuse.
enum ConcurrentOutcome {
	/// The stream opened and ran to its end, cleanly.
	Streamed,
	/// The stream never opened. The container may be missing or not yet
	/// created; the parent records it as a tolerated failure.
	OpenFailed(crate::libpod::PodmanError),
	/// The stream died while the container was still running, so live output
	/// was truncated (#1104).
	Truncated(crate::libpod::PodmanError),
}

impl Engine {
	/// Stream logs with `docker compose logs` options plus the prefix-display
	/// controls (`--no-color`, `--no-log-prefix`).
	///
	/// When `target_services` is empty, logs from every service are streamed;
	/// otherwise only the named services (an unknown name is an error).
	pub async fn logs_with_display(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		opts: LogsOptions,
		display: LogsDisplay,
	) -> Result<()> {
		validate_log_filters(&opts)?;
		let follow = opts.follow;
		// `--no-log-prefix` drops the `{service} | ` tag; `--no-color` forces a
		// monochrome prefix even on a colour-capable stdout.
		let query = log_query(&opts);
		for svc in target_services {
			if !file.services.contains_key(svc) {
				return Err(ComposeError::ServiceNotFound(svc.into()));
			}
		}
		let selected: std::collections::HashSet<&str> =
			target_services.iter().map(String::as_str).collect();
		let (targets, first_err) = self.resolve_log_targets(file, &selected).await;

		// A container that is simply not there yet is tolerated per-container so
		// the services that *do* exist still stream; that is deliberate. Anything
		// else (the socket refusing, a 500) is not a per-container fact, it is the
		// command failing, and `logs` reported exit 0 for it. Collected here and
		// returned at the end so every reachable container is still shown first.
		//
		// Nothing resolved and something went wrong: there is no partial result to
		// preserve, so the tolerance has nothing left to protect. This is separate
		// from #1104: nothing here classifies how a stream *ended*, the request
		// never opened.
		if targets.is_empty() {
			if let Some(e) = first_err {
				return Err(e);
			}
		}

		// Same rule one level down: a container that will not stream is tolerated
		// while another does, but every target failing is the command failing.
		let mut streamed_err: Option<ComposeError> = None;
		// A stream that truncated live output is NOT the same class as a container
		// that would not open one. The latter is tolerated while another streams;
		// the former means output was lost, so it must survive the `streamed_any`
		// shortcut below rather than being masked by a sibling that streamed fine
		// (#1104).
		let mut truncated_err: Option<ComposeError> = None;
		let mut streamed_any = false;

		// When follow=true, streams never end until containers stop. Run them
		// concurrently so multiple containers don't block each other.
		if follow && targets.len() > 1 {
			streamed_any = self
				.stream_concurrent_targets(
					targets,
					&query,
					&display,
					&mut truncated_err,
					&mut streamed_err,
				)
				.await;
		} else {
			for container_name in targets {
				// The open-fail arm leaves `streamed_any` untouched (mirrors the
				// pre-split `continue`), so a missing/not-yet-created container
				// is tolerated per-container and the sibling services still show.
				match self
					.stream_one_target(
						&container_name,
						&query,
						&display,
						follow,
						&mut truncated_err,
						&mut streamed_err,
					)
					.await
				{
					StreamOneOutcome::Streamed => streamed_any = true,
					StreamOneOutcome::Failed => {}
				}
			}
		}

		// Checked before the tolerance shortcut: output was lost, and another
		// container streaming cleanly does not make that untrue.
		if let Some(e) = truncated_err {
			return Err(e);
		}
		if streamed_any {
			return Ok(());
		}
		streamed_err.map_or(Ok(()), Err)
	}

	/// Resolve the live replica map for the project and build the
	/// (de-duplicated, ordered) target list for the selected services. Returns `(targets, first_err)`: `first_err` is `Some` only when
	/// the bulk `live_project_replicas_sorted` call itself failed; the caller
	/// then has the error to surface when no targets survive.
	///
	/// Each entry is the container name to stream `/logs` from. The libpod
	/// `/logs` endpoint always frames its body with 8-byte multiplexed
	/// headers (stdout/stderr channel byte + payload length + payload),
	/// including for TTY containers, so `is_tty` is no longer a parsing
	/// selector here (it used to choose between raw and multiplexed on
	/// the docker compat path). Resolved against the containers Podman
	/// actually has, not the static compose replica count: after a
	/// runtime `scale`/`up --scale` the file's count no longer matches the
	/// live replicas, so `logs` would otherwise miss every replica beyond
	/// the first. Falls back to the static names for a service absent from
	/// the map (the bulk helper does not see the compose file).
	///
	/// Hoisted out of the per-service loop so a `logs` over N services
	/// costs one container-list round-trip, not N (#1445), the same bulk
	/// path the per-service lifecycle commands took in #1363.
	///
	/// A failure resolving ONE service is tolerated so the others still
	/// print; that is deliberate and tested. The bulk GET is now the only
	/// point where a single engine-side failure can land, so the same
	/// tolerance collapses into "warn + remember, the post-loop empty
	/// check surfaces the error when there is no partial result to
	/// protect": an unreachable engine used to look like a project
	/// with no logs and still exit 0; this preserves that fix.
	///
	/// The skip-on-fetch-error case must NOT fall back to the static
	/// names below: the original per-service loop did `continue` on a
	/// resolution error, leaving `targets` empty so the post-loop check
	/// could surface the error. Falling back to static names would push
	/// phantom containers that 404 per-container, mask the failure, and
	/// make `logs` exit 0 on an unreachable engine again.
	async fn resolve_log_targets(
		&self,
		file: &ComposeFile,
		selected: &std::collections::HashSet<&str>,
	) -> (Vec<String>, Option<ComposeError>) {
		let mut first_err: Option<ComposeError> = None;
		let live_by_service = match self.live_project_replicas_sorted().await {
			Ok(by_service) => by_service,
			Err(e) => {
				tracing::warn!("logs: resolving project replicas: {e}");
				first_err.get_or_insert(e);
				std::collections::HashMap::new()
			}
		};
		let fetch_failed = first_err.is_some();
		let mut targets: Vec<String> = Vec::new();
		if !fetch_failed {
			for (n, s) in file
				.services
				.iter()
				.filter(|(n, _)| selected.is_empty() || selected.contains(n.as_str()))
			{
				let names = match live_by_service.get(n.as_str()) {
					Some(names) if !names.is_empty() => names.clone(),
					_ => self.replica_names(n, s),
				};
				for cname in names {
					targets.push(cname);
				}
			}
		}
		(targets, first_err)
	}

	/// Stream every target concurrently under `join_all`, returning
	/// `streamed_any` (true iff at least one container streamed).
	///
	/// The two failure classes are kept apart here and handed to the parent
	/// separately: a container that would not open a stream is tolerated
	/// while a sibling streams, while a stream that died while its container
	/// kept running truncated live output and no sibling can excuse that.
	/// Recording both as one error is what let a cleanly finished sibling
	/// cover the loss and report success.
	async fn stream_concurrent_targets(
		&self,
		targets: Vec<String>,
		query: &str,
		display: &LogsDisplay,
		truncated_err: &mut Option<ComposeError>,
		streamed_err: &mut Option<ComposeError>,
	) -> bool {
		let prefix = !display.no_log_prefix;
		let allow_color = !display.no_color;
		let futs: Vec<_> = targets
			.into_iter()
			.map(|container_name| {
				let client = &self.client;
				let query = query.to_string();
				async move {
					let path = format!(
						"{API_PREFIX}/containers/{}/logs?{query}",
						urlencoded(&container_name),
					);
					let resp = match client.get_stream(&path).await {
						Ok(r) => r,
						Err(e) => {
							tracing::warn!("logs {container_name}: {e}");
							return ConcurrentOutcome::OpenFailed(e);
						}
					};
					// The libpod `/logs` endpoint always frames its body with
					// 8-byte multiplexed headers (stdout/stderr channel byte +
					// payload length + payload), including for TTY containers.
					// The Docker compat handler used raw bytes for TTY
					// containers, so an `is_tty` selector was correct on the
					// compat path; on the libpod path it left the channel byte
					// (0x01 for stdout) on the first byte of every line and
					// rendered the stream unreadable. The raw path stays for
					// the hijacked attach/exec stream in `attach.rs`, which
					// goes to `/attach_websocket`, not `/logs`.
					let mut stream = crate::libpod::parse_multiplexed(resp.into_body());
					// These futures run concurrently under `join_all` on the
					// same task, so the stdout/stderr lock is taken and
					// released within each frame rather than held across the
					// `.await` above: holding a guard across the await would
					// let a sibling future block the thread on the same lock
					// and deadlock. Each frame still locks once and flushes,
					// keeping interleaved `logs -f` output prompt.
					let label = display_label(&container_name, &self.project);
					let mut out_pfx = LinePrefixer::new(&label, prefix, allow_color);
					let mut err_pfx = LinePrefixer::new(&label, prefix, allow_color);
					// Set when the stream ends with the container still
					// running: live output was truncated.
					let mut truncated: Option<crate::libpod::PodmanError> = None;
					while let Some(msg) = stream.next().await {
						let wrote = match msg {
							Ok(LogOutput::StdOut { message }) => {
								out_pfx.write(&mut std::io::stdout().lock(), &message)
							}
							Ok(LogOutput::StdErr { message }) => {
								err_pfx.write(&mut std::io::stderr().lock(), &message)
							}
							// A `logs -f` stream ends when its container stops, and
							// libpod marks that with a chunked terminator. A lost
							// terminator arrives here as an `Err` indistinguishable
							// at the transport layer from a real mid-stream break
							// (#1104), so resolve it out of band: if the container is
							// still running, live output was truncated and the
							// command failed.
							//
							// Measured on real 5.4.2 by restarting the libpod socket
							// under an attached `logs -f`: the arm is reached, the
							// container is still running, and this used to exit 0.
							// docker compose reports `unexpected EOF` and exits 1 on
							// the same failure, so 0 was a divergence, not parity.
							//
							// The re-check is point-in-time, so a genuine break that
							// coincides with the container stopping is knowingly
							// read as a clean end, because the transport cannot separate
							// them and the container is gone either way.
							Err(e) => {
								let kind = e.stream_end_kind();
								// The stream ended with an error. A frame the
								// daemon sent that does not parse is a failed
								// read whatever the container was doing; a
								// transport that died under a container that
								// keeps running lost live output (#1104). The
								// container's own state tells those two
								// apart.
								let still_running =
									self.container_still_running(&container_name).await;
								if stream_lost_output(&e, false, still_running) {
									tracing::warn!(
										"logs {container_name}: the stream ended before the \
										 output was complete [{kind}]: {e}"
									);
									truncated = Some(e);
								} else {
									tracing::warn!(
										"logs {container_name}: the stream ended as the \
										 container stopped [{kind}]"
									);
								}
								break;
							}
						};
						if stop_on_write_error(&container_name, wrote) {
							break;
						}
					}
					out_pfx.flush_tail(&mut std::io::stdout().lock());
					err_pfx.flush_tail(&mut std::io::stderr().lock());
					match truncated {
						Some(e) => ConcurrentOutcome::Truncated(e),
						None => ConcurrentOutcome::Streamed,
					}
				}
			})
			.collect();
		let mut streamed_any = false;
		for outcome in futures_util::future::join_all(futs).await {
			match outcome {
				ConcurrentOutcome::Streamed => streamed_any = true,
				ConcurrentOutcome::OpenFailed(e) => {
					streamed_err.get_or_insert(ComposeError::Podman(e));
				}
				ConcurrentOutcome::Truncated(e) => {
					// Output was lost. The container did stream, so it counts as
					// one, and the loss is recorded where the caller checks it
					// before the `streamed_any` shortcut.
					truncated_err.get_or_insert(ComposeError::Podman(e));
					streamed_any = true;
				}
			}
		}
		streamed_any
	}

	/// Stream one container on the sequential (single or first) path. Returns
	/// the outcome so `logs_with_display` can update `streamed_err` /
	/// `truncated_err` exactly once per container. Flushes the per-line
	/// prefixer before returning, regardless of how the inner loop ended, so
	/// the partial-line tail never sits in memory after the loop.
	async fn stream_one_target(
		&self,
		container_name: &str,
		query: &str,
		display: &LogsDisplay,
		follow: bool,
		truncated_err: &mut Option<ComposeError>,
		streamed_err: &mut Option<ComposeError>,
	) -> StreamOneOutcome {
		let prefix = !display.no_log_prefix;
		let allow_color = !display.no_color;
		let path = format!(
			"{API_PREFIX}/containers/{}/logs?{query}",
			urlencoded(container_name),
		);
		// Tolerate a missing/not-yet-created container the way the
		// multi-follow path does: warn and move on so the logs of the
		// services that *do* exist are still shown, instead of aborting the
		// whole command on the first 404.
		let resp = match self.client.get_stream(&path).await {
			Ok(r) => r,
			Err(e) => {
				tracing::warn!("logs {container_name}: {e}");
				streamed_err.get_or_insert(ComposeError::Podman(e));
				return StreamOneOutcome::Failed;
			}
		};
		// Same out-of-band resolution as the concurrent path above: the
		// libpod `/logs` endpoint always frames its body with 8-byte
		// multiplexed headers, including for TTY containers. The Docker
		// compat handler used raw bytes for TTY containers, so an `is_tty`
		// selector was correct on the compat path; on the libpod path it
		// stripped the leading channel byte (0x01) off every line. The raw
		// path stays for the hijacked attach/exec stream in `attach.rs`,
		// which goes to `/attach_websocket`, not `/logs`.
		let mut stream = crate::libpod::parse_multiplexed(resp.into_body());

		// Lock stdout once for the whole stream instead of re-acquiring
		// the lock (and issuing a syscall) per frame; stdout is ours
		// exclusively on this path. stderr is locked per frame because
		// the tracing subscriber also writes there: holding its lock
		// across the await loop would starve concurrent log emissions.
		// Flush after each frame so `logs -f` still streams promptly.
		let mut out = std::io::stdout().lock();
		let label = display_label(container_name, &self.project);
		let mut out_pfx = LinePrefixer::new(&label, prefix, allow_color);
		let mut err_pfx = LinePrefixer::new(&label, prefix, allow_color);
		// Set to the truncated outcome (if any) when the inner loop exits via
		// a stream-end Err; flushed at the bottom of the function regardless
		// of which arm the loop took.
		let mut ended_truncated: Option<ComposeError> = None;
		while let Some(msg) = stream.next().await {
			let wrote = match msg {
				Ok(LogOutput::StdOut { message }) => out_pfx.write(&mut out, &message),
				Ok(LogOutput::StdErr { message }) => {
					err_pfx.write(&mut std::io::stderr().lock(), &message)
				}
				// Same out-of-band resolution as the concurrent path above: a
				// stream that ends while its container is still running
				// truncated live output (#1104).
				Err(e) => {
					let kind = e.stream_end_kind();
					// Same out-of-band resolution as the concurrent path
					// above: a stream that ends while its container is still
					// running truncated live output (#1104). A bounded read
					// has no ambiguity to resolve: the history was supposed
					// to arrive in full, so a transport that dies under it
					// lost output, and a frame that does not parse is a
					// failed read whatever the container was doing.
					let still_running = if follow {
						self.container_still_running(container_name).await
					} else {
						None
					};
					if stream_lost_output(&e, !follow, still_running) {
						tracing::warn!(
							"logs {container_name}: the stream ended before the output \
							 was complete [{kind}]: {e}"
						);
						ended_truncated = Some(ComposeError::Podman(e));
					} else {
						tracing::warn!(
							"logs {container_name}: the stream ended as the container \
							 stopped [{kind}]"
						);
					}
					break;
				}
			};
			if stop_on_write_error(container_name, wrote) {
				break;
			}
		}
		out_pfx.flush_tail(&mut out);
		err_pfx.flush_tail(&mut std::io::stderr().lock());
		// The truncated arm reached the bottom of the per-container block in
		// the pre-split version (`truncated_err` was recorded and the loop
		// then fell through to flush + `streamed_any = true`). The helper
		// preserves both: `ended_truncated` was pinned here so the parent can
		// surface it via the post-loop reconciliation, and the caller's
		// match sets `streamed_any = true` for `Streamed`.
		if let Some(e) = ended_truncated {
			truncated_err.get_or_insert(e);
		}
		StreamOneOutcome::Streamed
	}
}

#[cfg(test)]
#[path = "with_display_tests.rs"]
mod tests;
