//! A crontab expression read into the sets of times it matches.
//!
//! The accepted subset is five fields (minute, hour, day of month, month, day
//! of week) built from `*`, numbers, lists, ranges, steps and three-letter
//! names, plus the macros `@hourly`, `@daily`, `@weekly`, `@monthly`,
//! `@yearly` and `@annually`. Anything else is an error that says why, because
//! a schedule that fires at other times than the cron line says is worse than
//! one that is not exported.

use std::collections::BTreeSet;

use super::field::{is_unrestricted, parse_field, DAY_OF_MONTH, DAY_OF_WEEK, HOUR, MINUTE, MONTH};

/// The times a cron expression matches, one set per field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Calendar {
	pub minutes: BTreeSet<u8>,
	pub hours: BTreeSet<u8>,
	pub days: BTreeSet<u8>,
	pub months: BTreeSet<u8>,
	/// Days of the week as 0 (Sunday) to 6 (Saturday).
	pub weekdays: BTreeSet<u8>,
}

/// Days in each month, February counted with its leap day.
const MAX_DAYS: [u8; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

impl Calendar {
	/// Whether no chosen day exists in any chosen month (`0 0 30 2 *`), so the
	/// schedule is valid and never fires. The day of week only narrows further,
	/// so it cannot make a firing calendar silent here.
	pub(super) fn never_fires(&self) -> bool {
		!self.months.iter().any(|month| {
			let longest = MAX_DAYS[usize::from(*month) - 1];
			self.days.iter().any(|day| *day <= longest)
		})
	}
}

/// Read `expression`, or say why it cannot be exported.
pub(super) fn parse_cron(expression: &str) -> Result<Calendar, String> {
	// The same whitelist the parser applies, repeated here because this is the
	// function the unit text is derived from.
	if !expression.chars().all(|c| {
		c.is_ascii_alphanumeric() || matches!(c, '*' | '/' | ',' | '-' | '?' | '#' | '@' | ' ')
	}) {
		return Err("it has a character outside 0-9, A-Z, a-z, * / , - ? # @ and space".into());
	}
	let expanded = expand_macro(expression.trim())?;
	let fields: Vec<&str> = expanded.split(' ').filter(|f| !f.is_empty()).collect();
	let [minute, hour, day, month, weekday] = fields[..] else {
		return Err(match fields.len() {
			6 | 7 => format!(
				"it has {} fields; only the five of crontab (no seconds, no year) are exported",
				fields.len()
			),
			n => format!("it has {n} fields, and crontab has five"),
		});
	};
	// With both day fields restricted cron fires when EITHER matches; systemd
	// fires only when BOTH do. No spelling of one gives the other.
	if !is_unrestricted(day) && !is_unrestricted(weekday) {
		return Err(
			"it restricts both the day of month and the day of week: cron fires when either \
			 matches and systemd only when both do"
				.into(),
		);
	}
	let weekdays = parse_field(weekday, &DAY_OF_WEEK)?
		.into_iter()
		.map(|d| d % 7)
		.collect();
	Ok(Calendar {
		minutes: parse_field(minute, &MINUTE)?,
		hours: parse_field(hour, &HOUR)?,
		days: parse_field(day, &DAY_OF_MONTH)?,
		months: parse_field(month, &MONTH)?,
		weekdays,
	})
}

/// The five-field form of a macro, or the text itself when it is not one.
fn expand_macro(text: &str) -> Result<String, String> {
	let Some(name) = text.strip_prefix('@') else {
		return Ok(text.to_string());
	};
	let fields = match name {
		"hourly" => "0 * * * *",
		"daily" => "0 0 * * *",
		"weekly" => "0 0 * * 0",
		"monthly" => "0 0 1 * *",
		"yearly" | "annually" => "0 0 1 1 *",
		"reboot" => return Err("`@reboot` has no calendar time to export".into()),
		_ => return Err(format!("`@{name}` is not one of the exported macros")),
	};
	Ok(fields.to_string())
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
