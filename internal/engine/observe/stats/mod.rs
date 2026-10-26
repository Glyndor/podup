//! `stats` is the live resource-usage stream for a project's service containers.

use std::collections::HashSet;

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::ContainerListEntry;
use crate::libpod::{parse_json_lines, urlencoded, API_PREFIX};

use crate::engine::Engine;

mod frame;
mod model;
mod options;
mod render;

use frame::{live_stats_allowed, print_frame, wants_region};
use model::StatsReport;
pub use options::StatsOptions;

impl Engine {
	/// Stream resource usage for the project's service containers (docker
	/// `compose stats`). Streams continuously until interrupted; `no_stream`
	/// prints a single snapshot. `target_services` narrows to specific services.
	pub async fn stats(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		no_stream: bool,
	) -> Result<()> {
		self.stats_with_options(
			file,
			target_services,
			StatsOptions {
				no_stream,
				..StatsOptions::default()
			},
		)
		.await
	}

	/// Stream resource usage with `docker compose stats`-style options:
	/// `--no-stream` (single snapshot), `-a/--all` (include non-running
	/// containers as zeroed rows), `--format` (table | json), and `--no-trunc`
	/// (keep full container names). `target_services` narrows to specific
	/// services.
	pub async fn stats_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		opts: StatsOptions,
	) -> Result<()> {
		// Reject unknown/typo service names instead of silently sampling the whole
		// host and printing a header-only table, matching the other commands.
		if let Some(unknown) = first_unknown_service(file, target_services) {
			return Err(ComposeError::ServiceNotFound(unknown.into()));
		}
		let targets = self.target_containers(file, target_services).await?;

		// Only running containers carry live samples, so scope the libpod
		// `containers=` filter to them: a stopped/created container fed to that
		// filter 404s the whole request. Non-running rows are synthesized locally
		// (as zeros) when `--all` is set.
		let running: HashSet<String> = targets
			.iter()
			.filter(|t| t.running)
			.map(|t| t.name.clone())
			.collect();
		let stopped: Vec<String> = if opts.all {
			targets
				.iter()
				.filter(|t| !t.running)
				.map(|t| t.name.clone())
				.collect()
		} else {
			Vec::new()
		};

		// Scope the stats stream to just the running containers server-side via the
		// `containers=` query param, so the daemon does not sample every container
		// on the host (the response is still filtered locally by `running`).
		let containers = containers_query(&running);

		if opts.no_stream || running.is_empty() {
			// Nothing running means nothing to sample (and an empty `containers=`
			// filter would otherwise fall back to the whole host), so skip the call
			// and render an empty/`--all`-only frame.
			let report = if running.is_empty() {
				StatsReport::default()
			} else {
				self.client
					.get_json(&format!(
						"{API_PREFIX}/containers/stats?stream=false{containers}"
					))
					.await
					.map_err(ComposeError::Podman)?
			};
			print_frame(&report, &running, &stopped, &opts, None)?;
			return Ok(());
		}

		let resp = self
			.client
			.get_stream(&format!(
				"{API_PREFIX}/containers/stats?stream=true{containers}"
			))
			.await
			.map_err(ComposeError::Podman)?;
		let mut frames = parse_json_lines::<StatsReport, _>(resp.into_body());

		// A live region only where one belongs: stdout a terminal, colour on, the
		// width readable, the same three conditions the lifecycle board uses,
		// asked of stdout rather than stderr because `stats` *is* its output. A
		// `--format json` stream is a machine path and never repaints.
		let mut region = wants_region(opts.json, live_stats_allowed())
			.then(|| crate::ui::progress::Region::new(crate::ui::progress::Target::Stdout));

		while let Some(frame) = frames.next().await {
			match frame {
				Ok(report) => print_frame(&report, &running, &stopped, &opts, region.as_mut())?,
				Err(e) => {
					// A `stats --stream` stream lives as long as any sampled
					// container is running, and ends once none remain. libpod
					// signals that end with a chunked terminator, but a lost
					// terminator (a dropped connection, or a version that omits it)
					// reaches here as an `Err` that is *indistinguishable* from a
					// real mid-sample break at the transport layer (#1104). Resolve
					// it out of band: re-check what is still running. If nothing the
					// stream was sampling is alive, the end was expected and the
					// command succeeded; if something is still running, the stream
					// truncated a live sample and the command failed, which is the
					// exit code a monitor scraping `stats` needs (#1080).
					//
					// The re-check is point-in-time: it samples state a moment after
					// the break, so a genuine break that happens to coincide with
					// every sampled container stopping is knowingly tolerated as a
					// clean end: the transport layer cannot tell the two apart, and
					// the sampled containers are gone either way.
					let still_running = match self.target_containers(file, target_services).await {
						Ok(targets) => targets
							.into_iter()
							.filter(|c| c.running)
							.map(|c| c.name)
							.collect::<HashSet<String>>(),
						// Fail closed: an unreadable running set is not confirmation
						// the end was expected, so the original error stands rather
						// than masking a possible failure. Decided here, at the point
						// of the inconclusive re-check, so the guarantee does not
						// depend on the sampled set being non-empty.
						Err(_) => {
							tracing::warn!(
								"stats: stream ended and the running set could not be \
								 re-checked [{}]: {e}",
								e.stream_end_kind()
							);
							return Err(ComposeError::Podman(e));
						}
					};
					if stats_stream_broke_mid_sample(&running, &still_running) {
						tracing::warn!(
							"stats: stream broke while a container was still running [{}]: {e}",
							e.stream_end_kind()
						);
						return Err(ComposeError::Podman(e));
					}
					tracing::debug!(
						"stats: stream ended as its containers stopped [{}]",
						e.stream_end_kind()
					);
					break;
				}
			}
		}
		Ok(())
	}

	/// The containers to report on: every existing replica of the targeted
	/// services (all services when `target_services` is empty), paired with
	/// whether each is currently running. Only containers that actually exist are
	/// returned (no static-name fallback): an absent service simply contributes
	/// no rows.
	async fn target_containers(
		&self,
		file: &ComposeFile,
		target_services: &[String],
	) -> Result<Vec<TargetContainer>> {
		let path = format!(
			"{API_PREFIX}/containers/json?all=true&filters={}",
			self.project_label_filter_encoded(),
		);
		let entries = self
			.client
			.get_json::<Vec<ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		let mut out = Vec::new();
		for e in entries {
			let service = e
				.labels
				.get("podup.service")
				.map(String::as_str)
				.unwrap_or("");
			// Skip containers whose service the compose file no longer defines, and
			// honour a positional `SERVICE` filter.
			if !file.services.contains_key(service) {
				continue;
			}
			if !target_services.is_empty() && !target_services.iter().any(|t| t == service) {
				continue;
			}
			if let Some(raw) = e.names.first() {
				out.push(TargetContainer {
					name: raw.trim_start_matches('/').to_string(),
					running: e.state == "running",
				});
			}
		}
		Ok(out)
	}
}

/// A project container considered for `stats`, with its run state so non-running
/// containers can be folded in (as zeroed rows) only under `--all`.
struct TargetContainer {
	name: String,
	running: bool,
}

/// The first targeted service name that the compose file does not define, if any.
/// Pure so the validation is unit-tested without a live Podman socket.
fn first_unknown_service<'a>(file: &ComposeFile, targets: &'a [String]) -> Option<&'a str> {
	targets
		.iter()
		.map(String::as_str)
		.find(|t| !file.services.contains_key(*t))
}

/// Build the query fragment scoping a stats request to the `wanted` containers,
/// or an empty string when none are wanted (which falls back to the daemon
/// default). libpod's `/containers/stats` expects the `containers` parameter
/// **repeated** once per container (`&containers=a&containers=b`), not a single
/// comma-joined value: a comma-joined list is parsed as one container name and
/// 404s. Names are sorted for a stable URL and each is URL-encoded.
fn containers_query(wanted: &HashSet<String>) -> String {
	if wanted.is_empty() {
		return String::new();
	}
	let mut names: Vec<&String> = wanted.iter().collect();
	names.sort();
	names
		.iter()
		.map(|n| format!("&containers={}", urlencoded(n)))
		.collect::<String>()
}

/// Whether a `stats --stream` stream that ended with an error broke while a
/// sampled container was still running.
///
/// The stream lives as long as any sampled container runs and ends once none
/// remain, so the error is a real failure only when the re-checked running set
/// still holds one of the containers the stream was sampling. When every
/// sampled container has stopped, the end was expected and the missing terminal
/// frame is the finished-vs-broken ambiguity (#1104), not a fault. Pure so the
/// decision is unit-tested without a live socket.
fn stats_stream_broke_mid_sample(
	sampled: &HashSet<String>,
	still_running: &HashSet<String>,
) -> bool {
	sampled.iter().any(|c| still_running.contains(c))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;
