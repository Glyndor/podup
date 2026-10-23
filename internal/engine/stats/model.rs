//! The libpod wire model for one `stats` frame.
//!
//! libpod answers `/containers/stats` with camel-cased JSON keys and a
//! `null` where a container has no interfaces, so every field carries its own
//! rename and default. Nothing here decides how a frame is shown; that is the
//! renderer's question, and the types are shared with it as plain data.

use std::collections::HashMap;

use serde::Deserialize;

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
pub(super) struct StatsReport {
	#[serde(rename = "Stats", default)]
	pub(super) stats: Vec<ContainerStat>,
}

/// Per-container resource sample within a [`StatsReport`].
#[derive(Deserialize, Default, Clone)]
pub(super) struct ContainerStat {
	#[serde(rename = "Name", default)]
	pub(super) name: String,
	#[serde(rename = "CPU", default)]
	pub(super) cpu: f64,
	#[serde(rename = "MemUsage", default)]
	pub(super) mem_usage: u64,
	#[serde(rename = "MemLimit", default)]
	pub(super) mem_limit: u64,
	#[serde(rename = "MemPerc", default)]
	pub(super) mem_perc: f64,
	#[serde(rename = "BlockInput", default)]
	pub(super) block_in: u64,
	#[serde(rename = "BlockOutput", default)]
	pub(super) block_out: u64,
	#[serde(rename = "PIDs", default)]
	pub(super) pids: u64,
	// `#[serde(default)]` also tolerates an explicit `null` frame value: libpod
	// sends `"network": null` for a container with no interfaces, which would
	// otherwise fail with `invalid type: null, expected a map`.
	#[serde(rename = "Network", default, deserialize_with = "null_default")]
	pub(super) network: HashMap<String, NetStat>,
}

/// Per-interface network counters.
#[derive(Deserialize, Default, Clone)]
pub(super) struct NetStat {
	#[serde(rename = "RxBytes", default)]
	pub(super) rx: u64,
	#[serde(rename = "TxBytes", default)]
	pub(super) tx: u64,
}
