//! Entry point for the fuzz harness: the same conversion `append_job_units`
//! runs for one `triggers.schedule` entry, without the unit around it.

use super::schedule::plan;
use crate::compose::types::{Schedule, ScheduleSpec};

/// What the conversion produced for one entry.
#[derive(Debug, PartialEq, Eq)]
pub struct Converted {
	/// The `OnCalendar=` text.
	pub calendar: String,
	/// The time zone that goes next to it, if the entry named one.
	pub timezone: Option<String>,
	/// Whether the calendar names no day that exists.
	pub never_fires: bool,
}

/// Convert `cron` (with an optional `timezone`) the way the exporter does, or
/// return the reason it would not be exported.
pub fn convert(cron: &str, timezone: Option<&str>) -> Result<Converted, String> {
	let schedule = match timezone {
		None => Schedule::Cron(cron.to_string()),
		Some(zone) => Schedule::Detailed(ScheduleSpec {
			cron: cron.to_string(),
			timezone: Some(zone.to_string()),
			concurrency: None,
			missed_fires: None,
		}),
	};
	plan(&schedule).map(|p| Converted {
		calendar: p.calendar,
		timezone: p.timezone,
		never_fires: p.never_fires,
	})
}
