use super::filters::health_from_status;
use super::format::{
	containers_path, display_status, table_created, table_service, table_size, table_status,
};
use super::json::ps_json_row;
use super::ports::{format_ports, publishers};
use super::*;
use crate::libpod::types::container::ContainerPort;
use std::collections::HashMap;

#[cfg(unix)]
use crate::engine::fake_podman;

#[cfg(unix)]
fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

fn entry(status: &str, state: &str) -> ContainerListEntry {
	ContainerListEntry {
		image_id: String::new(),
		id: "abc123".into(),
		names: vec!["/web".into()],
		image: "alpine".into(),
		status: status.into(),
		state: state.into(),
		ports: vec![],
		exit_code: None,
		labels: HashMap::new(),
		// Absent by default: the fixtures below exercise the fallback path, and
		// the ones that render an age set these explicitly.
		started_at: 0,
		created: String::new(),
		// Absent by default: libpod only fills this when the request asked, and
		// the tests that render the cell set it explicitly.
		size: None,
		// Absent on regular containers; infra containers set it explicitly.
		is_infra: false,
	}
}

/// A fixed `now` for the cells that render an age, so the tests are
/// deterministic. 2026-08-03 00:58:45 UTC, the instant captured from libpod.
const NOW: i64 = 1_785_718_725;

mod cells;
mod filters;
mod json;
mod ports;
mod request;
mod service_filter;
mod size;
mod table_status;
