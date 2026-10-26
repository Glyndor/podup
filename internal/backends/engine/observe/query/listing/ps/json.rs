//! The `ps --format json` row builder. Surfaces the fields docker compose
//! machine consumers expect (Service/State/Health/ExitCode/Publishers) in
//! addition to Name/Image/Status/ID. Pure so it can be unit-tested.

use crate::libpod::types::container::ContainerListEntry;

use super::filters::health_from_status;
use super::format::{display_status, name_of};
use super::ports::publishers;

/// Build one `ps --format json` row, surfacing the fields docker compose
/// machine consumers expect (Service/State/Health/ExitCode/Publishers) in
/// addition to Name/Image/Status/ID. Pure so it can be unit-tested.
pub(super) fn ps_json_row(c: &ContainerListEntry) -> serde_json::Value {
	serde_json::json!({
		"Name": name_of(c),
		"Image": c.image,
		"Project": c.labels.get("podup.project").cloned().unwrap_or_default(),
		"Service": c.labels.get("podup.service").cloned().unwrap_or_default(),
		"State": c.state,
		"Status": display_status(c),
		"Health": health_from_status(display_status(c)),
		"ExitCode": c.exit_code.unwrap_or(0),
		"Publishers": publishers(&c.ports),
		// The raw wire values, not a rendering. `docker compose ps --format
		// json` passes the RFC 3339 string through too, and a machine consumer
		// wants an instant it can compute with rather than `2mo 1d`.
		"Created": c.created,
		"StartedAt": c.started_at,
		// Raw byte counts, and null when the size was not requested, the same
		// distinction the table draws between an empty cell and a zero.
		"Size": c.size.map(|s| serde_json::json!({
			"RwSize": s.rw,
			"RootFsSize": s.root_fs,
		})),
		"ID": c.id,
	})
}
