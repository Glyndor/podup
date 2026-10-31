#![no_main]

//! Fuzz the crontab to `OnCalendar=` conversion that `podup quadlet` writes into
//! timer units. It must never panic and must be deterministic. Whatever it
//! exports must be a single line of the characters a calendar expression and a
//! time zone use, because the text lands in a unit file. For a purely numeric
//! cron it must also fire at exactly the times the cron line says: the emitted
//! calendar is read back and compared with an independent expansion of the cron.

use std::collections::BTreeSet;

use libfuzzer_sys::fuzz_target;

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn calendar_char(c: char) -> bool {
	c.is_ascii_alphanumeric() || matches!(c, '*' | '/' | ',' | ':' | '.' | '-' | ' ')
}

fn zone_char(c: char) -> bool {
	c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '/')
}

/// One cron field made only of digits, `*`, `,`, `-` and `/`, as a set.
fn cron_set(field: &str, min: u8, max: u8) -> Option<BTreeSet<u8>> {
	let mut out = BTreeSet::new();
	for item in field.split(',') {
		let (range, step) = match item.split_once('/') {
			Some((r, s)) => (r, s.parse::<usize>().ok().filter(|s| *s >= 1)?),
			None => (item, 1),
		};
		let (low, high) = if range == "*" {
			(min, max)
		} else if let Some((a, b)) = range.split_once('-') {
			(number(a, min, max)?, number(b, min, max)?)
		} else if step == 1 && !item.contains('/') {
			let n = number(range, min, max)?;
			(n, n)
		} else {
			return None;
		};
		if low > high {
			return None;
		}
		out.extend((low..=high).step_by(step));
	}
	Some(out)
}

fn number(text: &str, min: u8, max: u8) -> Option<u8> {
	if text.is_empty() || text.len() > 2 || !text.bytes().all(|b| b.is_ascii_digit()) {
		return None;
	}
	text.parse().ok().filter(|n| (min..=max).contains(n))
}

/// One calendar field the exporter writes: `*`, `a/step`, or a list of `a`,
/// `a,b` and `a..b`, as the set of values it names.
fn calendar_set(field: &str, min: u8, max: u8, names: &[&str]) -> Option<BTreeSet<u8>> {
	if field == "*" {
		return Some((min..=max).collect());
	}
	let value = |t: &str| -> Option<u8> {
		match names.iter().position(|n| *n == t) {
			Some(i) => u8::try_from(i).ok(),
			None => number(t, min, max),
		}
	};
	if let Some((start, step)) = field.split_once('/') {
		let (start, step) = (
			value(start)?,
			step.parse::<usize>().ok().filter(|s| *s >= 1)?,
		);
		return Some((start..=max).step_by(step).collect());
	}
	let mut out = BTreeSet::new();
	for item in field.split(',') {
		match item.split_once("..") {
			Some((a, b)) => out.extend(value(a)?..=value(b)?),
			None => {
				out.insert(value(item)?);
			}
		}
	}
	Some(out)
}

/// Check the emitted calendar against the cron line when the line is simple
/// enough to expand without repeating the converter.
fn check_meaning(cron: &str, calendar: &str) {
	let fields: Vec<&str> = cron.split_whitespace().collect();
	let [minute, hour, day, month, weekday] = fields[..] else {
		return;
	};
	let plain = |f: &str| {
		f.bytes()
			.all(|b| b.is_ascii_digit() || b"*,-/".contains(&b))
	};
	if !fields.iter().all(|f| plain(f)) {
		return;
	}
	let (Some(minutes), Some(hours), Some(days), Some(months), Some(weekdays)) = (
		cron_set(minute, 0, 59),
		cron_set(hour, 0, 23),
		cron_set(day, 1, 31),
		cron_set(month, 1, 12),
		cron_set(weekday, 0, 7),
	) else {
		return;
	};
	// Cron's Sunday is 0 or 7; the calendar names Monday first.
	let wanted_days: BTreeSet<u8> = weekdays.iter().map(|d| (d % 7 + 6) % 7).collect();

	let (names, rest) = match calendar.split_once(' ') {
		Some((first, rest))
			if first
				.chars()
				.next()
				.is_some_and(|c| c.is_ascii_alphabetic()) =>
		{
			(Some(first), rest)
		}
		_ => (None, calendar),
	};
	let got_days = match names {
		Some(names) => calendar_set(names, 0, 6, &WEEKDAYS)
			.unwrap_or_else(|| panic!("weekday part {names:?} of {calendar:?} is not readable")),
		None => (0..=6).collect(),
	};
	assert_eq!(got_days, wanted_days, "{cron:?} -> {calendar:?}: weekdays");

	let mut parts = rest.split(' ');
	let (Some(date), Some(time), None) = (parts.next(), parts.next(), parts.next()) else {
		panic!("{calendar:?} is not `date time`");
	};
	let date: Vec<&str> = date.splitn(3, '-').collect();
	let time: Vec<&str> = time.split(':').collect();
	let (["*", m, d], [h, mi, "00"]) = (&date[..], &time[..]) else {
		panic!("{calendar:?} does not have the shape `*-M-D h:m:00`");
	};
	for (label, text, want, min, max) in [
		("month", m, &months, 1, 12),
		("day", d, &days, 1, 31),
		("hour", h, &hours, 0, 23),
		("minute", mi, &minutes, 0, 59),
	] {
		let got = calendar_set(text, min, max, &[])
			.unwrap_or_else(|| panic!("{label} {text:?} of {calendar:?} is not readable"));
		assert_eq!(&got, want, "{cron:?} -> {calendar:?}: {label}");
	}
}

fuzz_target!(|data: &[u8]| {
	let Ok(text) = std::str::from_utf8(data) else {
		return;
	};
	let mut lines = text.splitn(2, '\n');
	let cron = lines.next().unwrap_or("");
	let timezone = lines.next();

	let first = podup::fuzz_api::cron_convert(cron, timezone);
	assert_eq!(
		first,
		podup::fuzz_api::cron_convert(cron, timezone),
		"not deterministic for {text:?}"
	);
	let Ok(out) = first else {
		return;
	};
	let calendar = &out.calendar;
	assert!(
		!calendar.is_empty() && calendar.len() <= 512,
		"length of {calendar:?}"
	);
	assert!(
		calendar.chars().all(calendar_char),
		"character outside the list in {calendar:?}"
	);
	assert!(
		!calendar.chars().any(char::is_control),
		"control character in {calendar:?}"
	);
	assert!(
		!calendar.starts_with(' ') && !calendar.ends_with(' ') && !calendar.contains("  "),
		"stray space in {calendar:?}"
	);
	if let Some(zone) = &out.timezone {
		assert!(
			zone.chars().all(zone_char),
			"character outside the list in zone {zone:?}"
		);
		assert!(!zone.contains(".."), "`..` in zone {zone:?}");
	}
	check_meaning(cron, calendar);
});
