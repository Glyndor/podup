//! `stats` — live resource-usage stream for a project's service containers.

use std::collections::{HashMap, HashSet};

use futures_util::StreamExt;
use serde::Deserialize;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::ContainerListEntry;
use crate::libpod::{parse_json_lines, urlencoded, API_PREFIX};

use super::Engine;

/// Options for [`Engine::stats_with_options`], mirroring `docker compose stats`
/// and the table-shaping flags the other list commands expose. Kept off the
/// frozen [`Engine::stats`] signature so the 1.0 library API stays stable.
#[derive(Default)]
pub struct StatsOptions {
	/// Disable streaming; print a single snapshot and exit, `--no-stream`.
	pub no_stream: bool,
	/// Include non-running containers as zeroed rows, `-a/--all`.
	pub all: bool,
	/// Emit JSON instead of the table, `--format json`.
	pub json: bool,
	/// Disable container-name truncation in the table, `--no-trunc`.
	pub no_trunc: bool,
}

impl StatsOptions {
	/// Build options from the four CLI flags, in `--no-stream`/`--all`/
	/// `--no-trunc`/`--format json` order. A terse constructor so the CLI keeps
	/// the field names (all `pub`) available for clarity while the dispatch site
	/// stays compact.
	pub fn new(no_stream: bool, all: bool, no_trunc: bool, json: bool) -> Self {
		Self {
			no_stream,
			all,
			no_trunc,
			json,
		}
	}
}

/// Width of the table NAME column; long names are truncated to this width (with
/// a trailing ellipsis) unless `--no-trunc` is given, so a long container name
/// no longer overflows and shifts every following column. Matches [`HEADER`].
const NAME_WIDTH: usize = 32;

/// Build the query fragment scoping a stats request to the `wanted` containers,
/// or an empty string when none are wanted (which falls back to the daemon
/// default). libpod's `/containers/stats` expects the `containers` parameter
/// **repeated** once per container (`&containers=a&containers=b`), not a single
/// comma-joined value — a comma-joined list is parsed as one container name and
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

/// Deserialize a map field, treating an explicit JSON `null` as the default
/// (empty) map. libpod sends `"Network": null` for a container with no
/// interfaces, which plain `#[serde(default)]` does not tolerate.
fn null_default<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
	D: serde::Deserializer<'de>,
	T: Default + Deserialize<'de>,
{
	Option::<T>::deserialize(d).map(|v| v.unwrap_or_default())
}

/// One frame of the libpod `/containers/stats` response.
#[derive(Deserialize, Default)]
struct StatsReport {
	#[serde(rename = "Stats", default)]
	stats: Vec<ContainerStat>,
}

/// Per-container resource sample within a [`StatsReport`].
#[derive(Deserialize, Default, Clone)]
struct ContainerStat {
	#[serde(rename = "Name", default)]
	name: String,
	#[serde(rename = "CPU", default)]
	cpu: f64,
	#[serde(rename = "MemUsage", default)]
	mem_usage: u64,
	#[serde(rename = "MemLimit", default)]
	mem_limit: u64,
	#[serde(rename = "MemPerc", default)]
	mem_perc: f64,
	#[serde(rename = "BlockInput", default)]
	block_in: u64,
	#[serde(rename = "BlockOutput", default)]
	block_out: u64,
	#[serde(rename = "PIDs", default)]
	pids: u64,
	// `#[serde(default)]` also tolerates an explicit `null` frame value: libpod
	// sends `"network": null` for a container with no interfaces, which would
	// otherwise fail with `invalid type: null, expected a map`.
	#[serde(rename = "Network", default, deserialize_with = "null_default")]
	network: HashMap<String, NetStat>,
}

/// Per-interface network counters.
#[derive(Deserialize, Default, Clone)]
struct NetStat {
	#[serde(rename = "RxBytes", default)]
	rx: u64,
	#[serde(rename = "TxBytes", default)]
	tx: u64,
}

/// Render a byte count as a compact human string (`1.5MiB`). Pure for testing.
fn format_bytes(bytes: u64) -> String {
	const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
	let mut value = bytes as f64;
	let mut unit = 0;
	while value >= 1024.0 && unit < UNITS.len() - 1 {
		value /= 1024.0;
		unit += 1;
	}
	if unit == 0 {
		format!("{bytes}B")
	} else {
		format!("{value:.1}{}", UNITS[unit])
	}
}

/// Sum a container's per-interface network counters into one `(rx, tx)` pair.
fn net_totals(s: &ContainerStat) -> (u64, u64) {
	s.network
		.values()
		.fold((0u64, 0u64), |(rx, tx), n| (rx + n.rx, tx + n.tx))
}

/// The NAME cell for the table: the full name when `no_trunc`, otherwise
/// truncated to [`NAME_WIDTH`] with a trailing ellipsis so a long name keeps the
/// row aligned. Counts characters (not bytes) so multi-byte names truncate
/// safely. Pure for testing.
fn truncate_name(name: &str, no_trunc: bool) -> String {
	if no_trunc || name.chars().count() <= NAME_WIDTH {
		return name.to_string();
	}
	let head: String = name.chars().take(NAME_WIDTH - 1).collect();
	format!("{head}…")
}

/// Format one stats row into the table layout. With `no_trunc` a long name is
/// left intact (and may overflow its column); otherwise it is truncated to
/// [`NAME_WIDTH`]. Pure for testing.
fn format_row(s: &ContainerStat, no_trunc: bool) -> String {
	let (rx, tx) = net_totals(s);
	format!(
		"{:<NAME_WIDTH$} {:>7.2}% {:>10} / {:<10} {:>6.2}% {:>9} / {:<9} {:>9} / {:<9} {:>5}",
		truncate_name(&s.name, no_trunc),
		s.cpu,
		format_bytes(s.mem_usage),
		format_bytes(s.mem_limit),
		s.mem_perc,
		format_bytes(rx),
		format_bytes(tx),
		format_bytes(s.block_in),
		format_bytes(s.block_out),
		s.pids,
	)
}

/// Build one `stats --format json` row with numeric values (raw bytes/percent),
/// so machine consumers get exact figures rather than the table's rounded,
/// human-formatted cells. Pure so it can be unit-tested.
fn stat_json_row(s: &ContainerStat) -> serde_json::Value {
	let (rx, tx) = net_totals(s);
	serde_json::json!({
		"Name": s.name,
		"CPUPerc": s.cpu,
		"MemUsage": s.mem_usage,
		"MemLimit": s.mem_limit,
		"MemPerc": s.mem_perc,
		"NetInput": rx,
		"NetOutput": tx,
		"BlockInput": s.block_in,
		"BlockOutput": s.block_out,
		"PIDs": s.pids,
	})
}

const HEADER: &str = "NAME                                 CPU %       MEM USAGE / LIMIT        MEM %    NET I/O             BLOCK I/O           PIDS";

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
			// filter would otherwise fall back to the whole host) — skip the call
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
			print_frame(&report, &running, &stopped, &opts);
			return Ok(());
		}

		let resp = self
			.client
			.get_stream(&format!(
				"{API_PREFIX}/containers/stats?stream=true{containers}"
			))
			.await
			.map_err(ComposeError::Podman)?;
		let mut frames = parse_json_lines::<StatsReport>(resp.into_body());
		while let Some(frame) = frames.next().await {
			match frame {
				Ok(report) => print_frame(&report, &running, &stopped, &opts),
				Err(e) => {
					tracing::debug!("stats stream ended: {e}");
					break;
				}
			}
		}
		Ok(())
	}

	/// The containers to report on — every existing replica of the targeted
	/// services (all services when `target_services` is empty), paired with
	/// whether each is currently running. Only containers that actually exist are
	/// returned (no static-name fallback): an absent service simply contributes
	/// no rows.
	async fn target_containers(
		&self,
		file: &ComposeFile,
		target_services: &[String],
	) -> Result<Vec<TargetContainer>> {
		let filters = serde_json::json!({ "label": [format!("podup.project={}", self.project)] });
		let path = format!(
			"{API_PREFIX}/containers/json?all=true&filters={}",
			urlencoded(&filters.to_string()),
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

/// Assemble the rows for one frame: the live samples for `running` containers
/// plus synthesized zero rows for each `stopped` container (already empty when
/// `--all` is off), sorted by name for stable output.
fn frame_rows(
	report: &StatsReport,
	running: &HashSet<String>,
	stopped: &[String],
) -> Vec<ContainerStat> {
	let mut rows: Vec<ContainerStat> = report
		.stats
		.iter()
		.filter(|s| running.contains(&s.name))
		.cloned()
		.collect();
	for name in stopped {
		rows.push(ContainerStat {
			name: name.clone(),
			..ContainerStat::default()
		});
	}
	rows.sort_by(|a, b| a.name.cmp(&b.name));
	rows
}

/// Print one stats frame: the table (a bold header plus one row per container)
/// or a JSON array when `--format json`. Table frames end with a blank line.
fn print_frame(
	report: &StatsReport,
	running: &HashSet<String>,
	stopped: &[String],
	opts: &StatsOptions,
) {
	let rows = frame_rows(report, running, stopped);

	if opts.json {
		let json: Vec<_> = rows.iter().map(stat_json_row).collect();
		println!(
			"{}",
			serde_json::to_string_pretty(&json).unwrap_or_default()
		);
		return;
	}

	crate::ui::print_bold_header(HEADER);
	for s in &rows {
		println!("{}", format_row(s, opts.no_trunc));
	}
	println!();
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn format_bytes_scales_units() {
		assert_eq!(format_bytes(512), "512B");
		assert_eq!(format_bytes(1024), "1.0KiB");
		assert_eq!(format_bytes(1536), "1.5KiB");
		assert_eq!(format_bytes(1024 * 1024), "1.0MiB");
		assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0GiB");
	}

	#[test]
	fn format_row_sums_network_and_shows_name() {
		let mut network = HashMap::new();
		network.insert("eth0".to_string(), NetStat { rx: 1024, tx: 2048 });
		let s = ContainerStat {
			name: "proj-web".into(),
			cpu: 12.5,
			mem_usage: 1024 * 1024,
			mem_limit: 1024 * 1024 * 1024,
			mem_perc: 0.1,
			block_in: 0,
			block_out: 0,
			pids: 3,
			network,
		};
		let row = format_row(&s, false);
		assert!(row.contains("proj-web"));
		assert!(row.contains("12.50%"));
		assert!(row.contains("1.0MiB"));
		assert!(row.contains('3'));
	}

	#[test]
	fn truncate_name_shortens_long_names_with_ellipsis() {
		let short = "proj-web-1";
		assert_eq!(truncate_name(short, false), short);
		// Exactly the column width is kept verbatim.
		let exact = "a".repeat(NAME_WIDTH);
		assert_eq!(truncate_name(&exact, false), exact);
		// One over the width is truncated to width-1 chars plus an ellipsis.
		let long = "a".repeat(NAME_WIDTH + 10);
		let cut = truncate_name(&long, false);
		assert_eq!(cut.chars().count(), NAME_WIDTH);
		assert!(cut.ends_with('…'));
		// `--no-trunc` keeps the full name regardless of length.
		assert_eq!(truncate_name(&long, true), long);
	}

	#[test]
	fn format_row_truncates_long_name_but_no_trunc_keeps_it() {
		let s = ContainerStat {
			name: "really-long-project-web-container-name-1".into(),
			..Default::default()
		};
		// Default: the long name is truncated (ellipsis present, full name gone).
		let row = format_row(&s, false);
		assert!(row.contains('…'));
		assert!(!row.contains(&s.name));
		// `--no-trunc`: the full name survives intact.
		let row_full = format_row(&s, true);
		assert!(row_full.contains(&s.name));
	}

	#[test]
	fn stat_json_row_emits_numeric_fields() {
		let mut network = HashMap::new();
		network.insert("eth0".to_string(), NetStat { rx: 100, tx: 200 });
		let s = ContainerStat {
			name: "proj-db-1".into(),
			cpu: 5.5,
			mem_usage: 2048,
			mem_limit: 4096,
			mem_perc: 50.0,
			block_in: 1,
			block_out: 2,
			pids: 7,
			network,
		};
		let row = stat_json_row(&s);
		assert_eq!(row["Name"], "proj-db-1");
		assert_eq!(row["CPUPerc"], 5.5);
		assert_eq!(row["MemUsage"], 2048);
		assert_eq!(row["MemLimit"], 4096);
		assert_eq!(row["NetInput"], 100);
		assert_eq!(row["NetOutput"], 200);
		assert_eq!(row["PIDs"], 7);
	}

	#[test]
	fn frame_rows_keeps_running_and_adds_stopped_zeros_sorted() {
		let report = StatsReport {
			stats: vec![
				ContainerStat {
					name: "proj-web-1".into(),
					cpu: 1.0,
					..Default::default()
				},
				// Not in `running` → must be dropped (stale daemon sample).
				ContainerStat {
					name: "other".into(),
					..Default::default()
				},
			],
		};
		let mut running = HashSet::new();
		running.insert("proj-web-1".to_string());
		let stopped = vec!["proj-db-1".to_string()];
		let rows = frame_rows(&report, &running, &stopped);
		// Sorted: db (stopped) before web (running); the unrelated sample is gone.
		assert_eq!(rows.len(), 2);
		assert_eq!(rows[0].name, "proj-db-1");
		assert_eq!(rows[0].cpu, 0.0);
		assert_eq!(rows[1].name, "proj-web-1");
		assert_eq!(rows[1].cpu, 1.0);
	}

	#[test]
	fn frame_rows_without_all_has_no_stopped_rows() {
		let report = StatsReport::default();
		let running = HashSet::new();
		// `--all` off → caller passes an empty `stopped` slice → no rows.
		let rows = frame_rows(&report, &running, &[]);
		assert!(rows.is_empty());
	}

	#[test]
	fn stat_tolerates_null_network() {
		// libpod sends `"Network": null` for a container with no interfaces; it
		// must deserialize to an empty map rather than erroring.
		let json = r#"{"Name":"proj-web","CPU":1.0,"Network":null}"#;
		let stat: ContainerStat = serde_json::from_str(json).unwrap();
		assert_eq!(stat.name, "proj-web");
		assert!(stat.network.is_empty());
	}

	#[test]
	fn stat_tolerates_missing_network() {
		let json = r#"{"Name":"proj-web"}"#;
		let stat: ContainerStat = serde_json::from_str(json).unwrap();
		assert!(stat.network.is_empty());
	}

	#[test]
	fn containers_query_repeats_param_per_container() {
		// libpod wants `containers` repeated, not comma-joined — a comma-joined
		// list is read as a single container name and 404s.
		let mut wanted = HashSet::new();
		wanted.insert("proj-web-1".to_string());
		wanted.insert("proj-db-1".to_string());
		assert_eq!(
			containers_query(&wanted),
			"&containers=proj-db-1&containers=proj-web-1"
		);
	}

	#[test]
	fn containers_query_empty_when_none_wanted() {
		assert_eq!(containers_query(&HashSet::new()), "");
	}

	#[test]
	fn first_unknown_service_flags_typos() {
		let file =
			crate::parse_str("services:\n  web:\n    image: nginx\n  db:\n    image: postgres\n")
				.unwrap();
		assert_eq!(first_unknown_service(&file, &[]), None);
		assert_eq!(
			first_unknown_service(&file, &["web".into(), "db".into()]),
			None
		);
		assert_eq!(
			first_unknown_service(&file, &["web".into(), "bogus".into()]),
			Some("bogus")
		);
	}
}
