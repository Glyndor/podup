//! The `OnCalendar=` text of a [`Calendar`].
//!
//! The text is built from the parsed sets and nothing else: digits, the
//! weekday names below and `* , . / : -` are the only characters that can
//! appear. [`is_safe_calendar`] checks that again on the finished text, so a
//! defect in the builder cannot reach a unit file.

use std::collections::BTreeSet;

use super::cron::Calendar;

/// Weekday names in systemd order, which starts on Monday.
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Longest `OnCalendar=` text accepted. The longest real one (every minute
/// and every hour listed) is well under this.
pub(super) const MAX_CALENDAR_BYTES: usize = 512;

/// The `OnCalendar=` value for `calendar`: `[Weekdays ]*-Month-Day Hour:Minute:00`.
pub(super) fn render(calendar: &Calendar) -> String {
	let mut text = String::new();
	// Cron numbers Sunday 0 and systemd lists it last; re-index to Monday first.
	let weekdays: BTreeSet<u8> = calendar.weekdays.iter().map(|d| (d + 6) % 7).collect();
	if weekdays.len() < 7 {
		text.push_str(&names(&weekdays));
		text.push(' ');
	}
	text.push_str(&format!(
		"*-{}-{} {}:{}:00",
		field(&calendar.months, 1, 12),
		field(&calendar.days, 1, 31),
		field(&calendar.hours, 0, 23),
		field(&calendar.minutes, 0, 59),
	));
	text
}

/// Weekday names, joined as a list with `Mon..Fri` for runs of three or more.
fn names(weekdays: &BTreeSet<u8>) -> String {
	runs(weekdays)
		.into_iter()
		.map(|(low, high)| match high - low {
			0 => WEEKDAYS[usize::from(low)].to_string(),
			1 => format!(
				"{},{}",
				WEEKDAYS[usize::from(low)],
				WEEKDAYS[usize::from(high)]
			),
			_ => format!(
				"{}..{}",
				WEEKDAYS[usize::from(low)],
				WEEKDAYS[usize::from(high)]
			),
		})
		.collect::<Vec<_>>()
		.join(",")
}

/// A numeric field: `*` for every value, `start/step` for an evenly spaced
/// series that runs to the end of the range, else a list with `a..b` runs.
fn field(values: &BTreeSet<u8>, min: u8, max: u8) -> String {
	if values.len() == usize::from(max - min) + 1 {
		return "*".to_string();
	}
	if let Some(series) = series(values, max) {
		return series;
	}
	runs(values)
		.into_iter()
		.map(|(low, high)| match high - low {
			0 => format!("{low:02}"),
			1 => format!("{low:02},{high:02}"),
			_ => format!("{low:02}..{high:02}"),
		})
		.collect::<Vec<_>>()
		.join(",")
}

/// `start/step` when `values` are `start, start + step, ...` and the next
/// step would pass `max`, which is what systemd generates for that spelling.
fn series(values: &BTreeSet<u8>, max: u8) -> Option<String> {
	let mut iter = values.iter().copied();
	let (first, second) = (iter.next()?, iter.next()?);
	let step = second - first;
	if step < 2 || values.len() < 3 {
		return None;
	}
	let expected: BTreeSet<u8> = (first..=max).step_by(usize::from(step)).collect();
	(&expected == values).then(|| format!("{first:02}/{step}"))
}

/// Maximal runs of consecutive values as `(low, high)` pairs.
fn runs(values: &BTreeSet<u8>) -> Vec<(u8, u8)> {
	let mut runs: Vec<(u8, u8)> = Vec::new();
	for &value in values {
		match runs.last_mut() {
			Some((_, high)) if *high + 1 == value => *high = value,
			_ => runs.push((value, value)),
		}
	}
	runs
}

/// Whether `text` has the shape [`render`] produces: only the characters of
/// a calendar expression, one line, bounded. Checked again on the text that is
/// about to be written, not only on the builder's input.
pub(super) fn is_safe_calendar(text: &str) -> bool {
	!text.is_empty()
		&& text.len() <= MAX_CALENDAR_BYTES
		&& text.chars().all(|c| {
			c.is_ascii_alphanumeric() || matches!(c, '*' | ',' | '.' | '/' | ':' | '-' | ' ')
		}) && !text.starts_with(' ')
		&& !text.ends_with(' ')
		&& !text.contains("  ")
}

#[cfg(test)]
#[path = "on_calendar_tests.rs"]
mod tests;
