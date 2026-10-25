//! One field of a crontab expression, read into the set of values it matches.
//!
//! Only the subset whose meaning is the same in cron and in systemd is read.
//! Every field becomes an explicit set of numbers, so the `OnCalendar=` text
//! written later describes that set and not a spelling of the original: a
//! construct this module does not know is an error, never a guess.

use std::collections::BTreeSet;

/// What a field accepts: the range, and the three-letter names it may use.
pub(super) struct FieldSpec {
	/// Name used in the reasons given back to the operator.
	pub label: &'static str,
	/// Smallest value.
	pub min: u8,
	/// Largest value. The day of the week allows 7 (Sunday) on top of 0..=6.
	pub max: u8,
	/// Names for `min`, `min + 1`, ...: `jan`..`dec` or `sun`..`sat`.
	pub names: &'static [&'static str],
	/// Whether a bare `?` is read as `*` (day of month and day of week only).
	pub allows_question: bool,
}

pub(super) const MINUTE: FieldSpec = FieldSpec {
	label: "minute",
	min: 0,
	max: 59,
	names: &[],
	allows_question: false,
};
pub(super) const HOUR: FieldSpec = FieldSpec {
	label: "hour",
	min: 0,
	max: 23,
	names: &[],
	allows_question: false,
};
pub(super) const DAY_OF_MONTH: FieldSpec = FieldSpec {
	label: "day of month",
	min: 1,
	max: 31,
	names: &[],
	allows_question: true,
};
pub(super) const MONTH: FieldSpec = FieldSpec {
	label: "month",
	min: 1,
	max: 12,
	names: &[
		"jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
	],
	allows_question: false,
};
pub(super) const DAY_OF_WEEK: FieldSpec = FieldSpec {
	label: "day of week",
	min: 0,
	max: 7,
	names: &["sun", "mon", "tue", "wed", "thu", "fri", "sat"],
	allows_question: true,
};

/// Whether the field text places no restriction: a bare `*` or `?`.
///
/// This is the test cron itself applies to decide how the day of month and the
/// day of week combine, so `1-31` is a restriction even though it matches
/// every day.
pub(super) fn is_unrestricted(text: &str) -> bool {
	text == "*" || text == "?"
}

/// The values `text` matches, or the reason it cannot be exported.
pub(super) fn parse_field(text: &str, spec: &FieldSpec) -> Result<BTreeSet<u8>, String> {
	if text.is_empty() {
		return Err(format!("the {} field is empty", spec.label));
	}
	if text == "?" {
		return if spec.allows_question {
			Ok((spec.min..=spec.max).collect())
		} else {
			Err(format!("`?` is not valid in the {} field", spec.label))
		};
	}
	let mut values = BTreeSet::new();
	for item in text.split(',') {
		values.extend(parse_item(item, spec)?);
	}
	Ok(values)
}

/// One comma-separated item: `*`, `n`, `a-b`, `*/s` or `a-b/s`.
fn parse_item(item: &str, spec: &FieldSpec) -> Result<Vec<u8>, String> {
	let (range, step) = match item.split_once('/') {
		Some((range, step)) => (range, Some(parse_step(step, spec)?)),
		None => (item, None),
	};
	let (low, high) = if range == "*" {
		(spec.min, spec.max)
	} else if let Some((low, high)) = range.split_once('-') {
		let (low, high) = (parse_value(low, spec)?, parse_value(high, spec)?);
		if low > high {
			return Err(format!(
				"the {} range `{range}` runs backwards, which cron and systemd read differently",
				spec.label
			));
		}
		(low, high)
	} else {
		let value = parse_value(range, spec)?;
		if step.is_some() {
			// `5/15` is "from 5 every 15" in some crons and a plain 5 in others.
			return Err(format!(
				"`{item}` in the {} field has a step on a single value, which crons read \
				 differently",
				spec.label
			));
		}
		(value, value)
	};
	Ok((low..=high)
		.step_by(usize::from(step.unwrap_or(1)))
		.collect())
}

/// A step: digits, from 1 up to the width of the field.
fn parse_step(text: &str, spec: &FieldSpec) -> Result<u8, String> {
	let step = parse_number(text)
		.filter(|step| *step >= 1 && *step <= spec.max - spec.min)
		.ok_or_else(|| {
			format!(
				"the step `{text}` in the {} field is not between 1 and {}",
				spec.label,
				spec.max - spec.min
			)
		})?;
	Ok(step)
}

/// A number, or a month or weekday name, inside the field's range.
fn parse_value(text: &str, spec: &FieldSpec) -> Result<u8, String> {
	let value = match parse_number(text) {
		Some(number) => Some(number),
		None => {
			let lower = text.to_ascii_lowercase();
			spec.names
				.iter()
				.position(|name| *name == lower)
				.and_then(|index| u8::try_from(index).ok())
				.map(|index| index + spec.min)
		}
	};
	match value {
		Some(value) if (spec.min..=spec.max).contains(&value) => Ok(value),
		_ => Err(format!(
			"`{text}` is not a valid value for the {} field ({} to {})",
			spec.label, spec.min, spec.max
		)),
	}
}

/// Up to two ASCII digits and nothing else: no sign, no space, no `0x`.
fn parse_number(text: &str) -> Option<u8> {
	if text.is_empty() || text.len() > 2 || !text.bytes().all(|b| b.is_ascii_digit()) {
		return None;
	}
	text.parse().ok()
}

#[cfg(test)]
#[path = "field_tests.rs"]
mod tests;
