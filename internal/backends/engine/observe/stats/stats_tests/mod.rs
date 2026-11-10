//! Unit tests for `stats`, split out to keep the module inside the source line
//! limit, following the same `tests.rs` split used by `autostart` and the libpod
//! client.
//!
//! The suite was a single `stats_tests.rs` over the 300-line soft limit, so it
//! became a folder: this module keeps the shared fixture and the index, and
//! each leaf holds the scenarios of one subject.

use std::collections::{HashMap, HashSet};

use super::frame::{frame_rows, wants_region};
use super::model::{ContainerStat, NetStat, StatsReport};
use super::render::{
	format_bytes, format_row_with, header, load_style, stat_json_row, truncate_name, NAME_WIDTH,
};
use super::{containers_query, first_unknown_service, stats_stream_broke_mid_sample};

/// A representative row: values wide enough that a column mis-sized by a couple
/// of characters shows up, rather than a zeroed struct where every cell happens
/// to be short.
fn sample_stat() -> ContainerStat {
	let mut network = HashMap::new();
	network.insert("eth0".to_string(), NetStat { rx: 1024, tx: 2048 });
	ContainerStat {
		name: "proj-web-1".into(),
		cpu: 12.5,
		mem_usage: 1024 * 1024,
		mem_limit: 60 * 1024 * 1024 * 1024,
		mem_perc: 0.06,
		block_in: 4096,
		block_out: 8192,
		pids: 3,
		network,
	}
}

mod bytes;
mod frame;
mod header;
mod json;
mod model;
mod network;
mod query;
mod row;
mod stream;
mod style;
