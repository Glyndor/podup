//! What one `triggers.schedule` entry becomes, or why it becomes nothing.

use super::cron::parse_cron;
use super::on_calendar;
use super::timer::is_safe_timezone;
use crate::compose::types::{Concurrency, MissedFires, Schedule};

/// A schedule entry that has a faithful systemd timer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plan {
	/// The `OnCalendar=` text, without time zone.
	pub calendar: String,
	/// IANA time zone, when the entry names one.
	pub timezone: Option<String>,
	/// `Persistent=`: catch up once after downtime (`missed_fires: one`).
	pub persistent: bool,
	/// The calendar has no day that exists in any chosen month.
	pub never_fires: bool,
}

/// The timer for `schedule`, or the reason it cannot be exported.
pub(super) fn plan(schedule: &Schedule) -> Result<Plan, String> {
	let mut timezone = None;
	let mut persistent = true;
	if let Schedule::Detailed(spec) = schedule {
		if spec.concurrency == Some(Concurrency::Queue) {
			return Err(
				"`concurrency: queue` has no systemd equivalent: a timer does not queue a fire \
				 while the previous run is still active, it drops it"
					.into(),
			);
		}
		persistent = spec.missed_fires != Some(MissedFires::Skip);
		if let Some(zone) = &spec.timezone {
			if !is_safe_timezone(zone) {
				return Err(
					"its `timezone` is not an IANA name (letters, digits and _ + - / only, \
					 no `..`)"
						.into(),
				);
			}
			timezone = Some(zone.clone());
		}
	}
	let parsed = parse_cron(schedule.cron())?;
	let calendar = on_calendar::render(&parsed);
	Ok(Plan {
		calendar,
		timezone,
		persistent,
		never_fires: parsed.never_fires(),
	})
}

#[cfg(test)]
#[path = "schedule_tests.rs"]
mod tests;
