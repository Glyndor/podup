//! Compose-shaped NDJSON, deliberately separate from the stable podup JSON array.

use super::format::{display_status, name_of, table_created, table_service, table_size};
use super::ports::{format_ports, publishers};
use super::{filters::health_from_status, ContainerListEntry};
use crate::error::Result;

fn row(c: &ContainerListEntry, now: i64) -> serde_json::Value {
	let mut labels: Vec<_> = c.labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
	labels.sort();
	serde_json::json!({
		// ContainerListEntry has no command or network names. Compose's string
		// fields stay empty rather than inventing values. Missing size is also
		// an empty string; Compose uses null for Publishers when there are no ports.
		"Command": "",
		"CreatedAt": c.created,
		"ExitCode": c.exit_code.unwrap_or(0),
		"Health": health_from_status(display_status(c)),
		"ID": c.id,
		"Image": c.image,
		"Labels": labels.join(","),
		"Name": name_of(c),
		"Names": c.names.iter().map(|name| name.trim_start_matches('/')).collect::<Vec<_>>().join(","),
		"Networks": "",
		"Ports": format_ports(&c.ports),
		"Project": c.labels.get("podup.project").cloned().unwrap_or_default(),
		"Publishers": (!c.ports.is_empty()).then(|| publishers(&c.ports)),
		"RunningFor": table_created(c, now),
		"Service": table_service(c),
		"Size": table_size(c),
		"State": c.state,
		"Status": display_status(c),
	})
}

pub(super) fn render(containers: &[ContainerListEntry], now: i64) -> Result<String> {
	let mut sorted: Vec<_> = containers.iter().collect();
	sorted.sort_by_key(|c| name_of(c));
	let mut output = String::new();
	for c in sorted {
		let line = serde_json::to_string(&row(c, now))
			.map_err(|e| crate::error::ComposeError::Unsupported(format!("ps.ndjson: {e}")))?;
		output.push_str(&line);
		output.push('\n');
	}
	Ok(output)
}
