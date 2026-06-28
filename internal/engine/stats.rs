//! `stats` — live resource-usage stream for a project's service containers.

use std::collections::{HashMap, HashSet};

use futures_util::StreamExt;
use serde::Deserialize;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{parse_json_lines, urlencoded, API_PREFIX};

use super::Engine;

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
#[derive(Deserialize)]
struct StatsReport {
	#[serde(rename = "Stats", default)]
	stats: Vec<ContainerStat>,
}

/// Per-container resource sample within a [`StatsReport`].
#[derive(Deserialize, Default)]
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
#[derive(Deserialize, Default)]
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

/// Format one stats row into the table layout. Pure for testing.
fn format_row(s: &ContainerStat) -> String {
	let (rx, tx) = s
		.network
		.values()
		.fold((0u64, 0u64), |(rx, tx), n| (rx + n.rx, tx + n.tx));
	format!(
		"{:<32} {:>7.2}% {:>10} / {:<10} {:>6.2}% {:>9} / {:<9} {:>9} / {:<9} {:>5}",
		s.name,
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
		let wanted = self.target_container_names(file, target_services).await?;

		// Scope the stats stream to just the wanted containers server-side via the
		// `containers=` query param, so the daemon does not sample every container
		// on the host (the response is still filtered locally by `wanted`).
		let containers = containers_query(&wanted);

		if no_stream {
			let report: StatsReport = self
				.client
				.get_json(&format!(
					"{API_PREFIX}/containers/stats?stream=false{containers}"
				))
				.await
				.map_err(ComposeError::Podman)?;
			print_frame(&report, &wanted);
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
				Ok(report) => print_frame(&report, &wanted),
				Err(e) => {
					tracing::debug!("stats stream ended: {e}");
					break;
				}
			}
		}
		Ok(())
	}

	/// The set of container names to report on: every replica of the targeted
	/// services (all services when `target_services` is empty).
	async fn target_container_names(
		&self,
		file: &ComposeFile,
		target_services: &[String],
	) -> Result<HashSet<String>> {
		let mut wanted = HashSet::new();
		for (name, service) in &file.services {
			if target_services.is_empty() || target_services.iter().any(|t| t == name) {
				for c in self.live_replica_names(name, service).await? {
					wanted.insert(c);
				}
			}
		}
		Ok(wanted)
	}
}

/// Print one stats frame: a header plus a row per wanted container (sorted for
/// stable output). Frames are separated by a blank line.
fn print_frame(report: &StatsReport, wanted: &HashSet<String>) {
	let mut rows: Vec<&ContainerStat> = report
		.stats
		.iter()
		.filter(|s| wanted.contains(&s.name))
		.collect();
	rows.sort_by(|a, b| a.name.cmp(&b.name));

	crate::ui::print_bold_header(HEADER);
	for s in rows {
		println!("{}", format_row(s));
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
		let row = format_row(&s);
		assert!(row.contains("proj-web"));
		assert!(row.contains("12.50%"));
		assert!(row.contains("1.0MiB"));
		assert!(row.contains('3'));
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
}
