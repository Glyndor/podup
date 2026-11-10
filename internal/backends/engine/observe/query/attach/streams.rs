use futures_util::stream::FuturesUnordered;

use super::abort::finish_abort;
use super::follow::Follow;
use super::phase::{wait_for_phase, Phase};
use super::select;
use super::{AttachOptions, AttachOutcome, AttachSummary, Engine};
use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

impl Engine {
	/// Like [`Engine::attach_logs`] but with `up --timestamps` and
	/// `--abort-on-container-exit` / `--exit-code-from` support.
	///
	/// `options.timestamps` prefixes each streamed line with the libpod
	/// RFC3339 timestamp.
	///
	/// `options.abort_on_container_exit` (and `options.exit_code_from`, which
	/// implies it) makes the call return
	/// [`AttachSummary::outcome`] = [`AttachOutcome::Aborted`] as soon as any
	/// container exits, with the propagating service name and exit code in
	/// the sibling fields of the returned [`AttachSummary`]. On that path the
	/// remaining containers are stopped before the function returns, so the
	/// caller does not need to call [`Engine::stop`], and `dispatch.rs` skips
	/// its own stop call on that outcome for the same reason.
	///
	/// The two abort-specific fields live on [`AttachSummary`] rather than as
	/// struct fields of [`AttachOutcome::Aborted`]: keeping the enum all-unit
	/// preserves the `Copy` derive it had in 4.0.0 and keeps the defined
	/// discriminants every `as isize` / `mem::discriminant` caller relies on.
	/// Callers that just need to know which ending happened can still
	/// pattern-match `summary.outcome` exactly as they did on 4.0.0; the
	/// sibling fields are only meaningful when the outcome is `Aborted`.
	///
	/// A service name passed as `options.exit_code_from` must exist in the
	/// compose file; a missing service is rejected with
	/// [`ComposeError::ServiceNotFound`] before any work happens (matching
	/// `docker compose` v5.1.3).
	pub async fn attach_logs_with(
		&self,
		file: &ComposeFile,
		options: &AttachOptions,
	) -> Result<AttachSummary> {
		// Reject `--exit-code-from` naming an unknown service up front. Doing this
		// before any container is created means a typo surfaces as a clear
		// "service X not found" error, matching docker compose v5.1.3.
		if let Some(target) = options.exit_code_from.as_deref() {
			if !file.services.contains_key(target) {
				return Err(ComposeError::ServiceNotFound(target.to_string()));
			}
		}

		let timestamps = options.timestamps;
		let abort_on_container_exit =
			options.abort_on_container_exit || options.exit_code_from.is_some();
		let abort_on_container_failure = options.abort_on_container_failure;
		let no_log_prefix = options.no_log_prefix;

		// Carry (service, display_name, container_name) so the abort path can map
		// a stream end back to the compose service that owns it without
		// re-deriving the project prefix.
		let attached: Vec<(String, String, String)> = select::attached_services(file, options)?
			.into_iter()
			.flat_map(|(name, s)| {
				let proj_prefix = format!("{}-", self.project);
				self.replica_names(name, s).into_iter().map(move |cname| {
					let display = cname
						.strip_prefix(proj_prefix.as_str())
						.map(|s| s.to_string())
						.unwrap_or_else(|| cname.clone());
					(name.to_string(), display, cname)
				})
			})
			.collect();

		if attached.is_empty() {
			// Nothing to stream is not an interruption.
			return Ok(AttachSummary {
				outcome: AttachOutcome::StreamsEnded,
				service: None,
				exit_code: None,
			});
		}

		let streams: FuturesUnordered<_> = attached
			.iter()
			.map(|(svc, display, cname)| {
				self.follow_container(Follow {
					service: svc,
					display,
					container: cname,
					timestamps,
					no_log_prefix,
					check_failure: abort_on_container_failure && !abort_on_container_exit,
				})
			})
			.collect();

		// Which arm wins is the whole point: `docker compose up` exits 130 on both
		// SIGINT and SIGTERM (measured against v5.1.3, not assumed: it is 130 for
		// SIGTERM too, not the 143 the signal number would suggest), and podup
		// exited 0 for both. A CI job that runs `up` in the foreground and is
		// cancelled therefore reported success.
		//
		// `FuturesUnordered` (instead of `join_all`) is what makes
		// `--abort-on-container-exit` work: we yield on the FIRST stream to
		// finish, then decide whether to keep waiting or stop the rest. With
		// `join_all` we would have to wait for every container, and a service
		// that exits cleanly could not trigger an abort while others were still
		// running.
		let phase =
			wait_for_phase(streams, abort_on_container_exit, abort_on_container_failure).await;

		match phase {
			Phase::Interrupted => Ok(AttachSummary {
				outcome: AttachOutcome::Interrupted,
				service: None,
				exit_code: None,
			}),
			Phase::AllEnded { saw_break } => {
				let outcome = if saw_break {
					AttachOutcome::StreamBroke
				} else {
					AttachOutcome::StreamsEnded
				};
				Ok(AttachSummary {
					outcome,
					service: None,
					exit_code: None,
				})
			}
			Phase::FirstExited {
				trigger_service,
				trigger_container,
			} => {
				let service = options
					.exit_code_from
					.as_deref()
					.unwrap_or(&trigger_service);
				let selection = self
					.abort_exit_container(file, service, &trigger_service, &trigger_container)
					.await;
				let exit_code = finish_abort(
					async {
						let container = selection?;
						self.container_exit_code(service, &container).await
					},
					self.stop(file, &[]),
				)
				.await?;

				Ok(AttachSummary {
					outcome: AttachOutcome::Aborted,
					service: Some(service.to_string()),
					exit_code: Some(exit_code),
				})
			}
		}
	}
}
