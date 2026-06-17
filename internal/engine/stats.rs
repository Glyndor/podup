//! `stats` — live resource-usage stream for a project's service containers.

use std::collections::{HashMap, HashSet};

use futures_util::StreamExt;
use serde::Deserialize;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{parse_json_lines, API_PREFIX};

use super::Engine;

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
	#[serde(rename = "Network", default)]
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
		let wanted = self.target_container_names(file, target_services);

		if no_stream {
			let report: StatsReport = self
				.client
				.get_json(&format!("{API_PREFIX}/containers/stats?stream=false"))
				.await
				.map_err(ComposeError::Podman)?;
			print_frame(&report, &wanted);
			return Ok(());
		}

		let resp = self
			.client
			.get_stream(&format!("{API_PREFIX}/containers/stats?stream=true"))
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
	fn target_container_names(
		&self,
		file: &ComposeFile,
		target_services: &[String],
	) -> HashSet<String> {
		file.services
			.iter()
			.filter(|(name, _)| {
				target_services.is_empty() || target_services.iter().any(|t| t == *name)
			})
			.flat_map(|(name, service)| self.replica_names(name, service))
			.collect()
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

	println!("{HEADER}");
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
}
