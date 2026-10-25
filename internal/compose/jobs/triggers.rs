//! Validation of a job's `triggers:` block.
//!
//! `triggers` is mandatory and must declare at least one type. The two types
//! are `manual` and `schedule`. podup validates the shape of a schedule and
//! never fires it, so every check here is about structure and about keeping
//! hostile text out of the strings that a later step writes into other files.

use serde_yaml::{Mapping, Value};

use super::parse::job_error;
use crate::compose::types::{Concurrency, MissedFires, Schedule, ScheduleSpec, Triggers};
use crate::error::{ComposeError, Result};

/// Longest crontab expression accepted. A real expression is a few dozen
/// bytes; the cap bounds what is stored, printed and later written to a unit.
pub(super) const MAX_CRON_BYTES: usize = 256;
/// Longest time zone name accepted. IANA names stay well under this.
pub(super) const MAX_TIMEZONE_BYTES: usize = 64;
/// Most schedule entries one job may declare.
pub(super) const MAX_SCHEDULES: usize = 64;

/// Parse the `triggers:` value of the job called `label` (already escaped).
pub(super) fn parse(label: &str, raw: &Value) -> Result<Triggers> {
	let Value::Mapping(map) = raw else {
		return Err(job_error(format!(
			"`triggers` of job '{label}' must be a mapping with `manual` and/or `schedule`"
		)));
	};
	let mut triggers = Triggers::default();
	for (key, value) in map {
		match key.as_str() {
			Some("manual") => triggers.manual = Some(parse_manual(label, value)?),
			Some("schedule") => triggers.schedule = parse_schedule(label, value)?,
			_ => {
				return Err(job_error(format!(
					"`triggers` of job '{label}' has an unknown trigger type; the supported \
					 types are `manual` and `schedule`"
				)))
			}
		}
	}
	if triggers.manual.is_none() && triggers.schedule.is_empty() {
		return Err(job_error(format!(
			"job '{label}' declares no trigger; `triggers` needs `manual: true` or at least \
			 one `schedule` entry"
		)));
	}
	Ok(triggers)
}

fn parse_manual(label: &str, value: &Value) -> Result<bool> {
	value.as_bool().ok_or_else(|| {
		job_error(format!(
			"`triggers.manual` of job '{label}' must be true or false"
		))
	})
}

fn parse_schedule(label: &str, value: &Value) -> Result<Vec<Schedule>> {
	let Value::Sequence(entries) = value else {
		return Err(job_error(format!(
			"`triggers.schedule` of job '{label}' must be a list"
		)));
	};
	if entries.len() > MAX_SCHEDULES {
		return Err(job_error(format!(
			"`triggers.schedule` of job '{label}' has more than {MAX_SCHEDULES} entries"
		)));
	}
	entries
		.iter()
		.enumerate()
		.map(|(index, entry)| parse_entry(label, index + 1, entry))
		.collect()
}

fn parse_entry(label: &str, position: usize, entry: &Value) -> Result<Schedule> {
	match entry {
		Value::String(cron) => Ok(Schedule::Cron(check_cron(label, position, cron)?)),
		Value::Mapping(map) => parse_spec(label, position, map).map(Schedule::Detailed),
		_ => Err(job_error(format!(
			"schedule entry {position} of job '{label}' must be a crontab string or a mapping \
			 with `cron`"
		))),
	}
}

fn parse_spec(label: &str, position: usize, map: &Mapping) -> Result<ScheduleSpec> {
	let mut cron = None;
	let mut timezone = None;
	let mut concurrency = None;
	let mut missed_fires = None;
	for (key, value) in map {
		let Some(key) = key.as_str() else {
			return Err(unknown_key(label, position));
		};
		match key {
			"cron" => {
				cron = Some(check_cron(
					label,
					position,
					text(label, position, key, value)?,
				)?)
			}
			"timezone" => {
				let zone = text(label, position, key, value)?;
				timezone = Some(check_timezone(label, position, zone)?);
			}
			"concurrency" => {
				concurrency = Some(match text(label, position, key, value)? {
					"forbid" => Concurrency::Forbid,
					"queue" => Concurrency::Queue,
					_ => return Err(bad_choice(label, position, key, "forbid or queue")),
				});
			}
			"missed_fires" => {
				missed_fires = Some(match text(label, position, key, value)? {
					"one" => MissedFires::One,
					"skip" => MissedFires::Skip,
					_ => return Err(bad_choice(label, position, key, "one or skip")),
				});
			}
			_ => return Err(unknown_key(label, position)),
		}
	}
	let cron = cron.ok_or_else(|| {
		job_error(format!(
			"schedule entry {position} of job '{label}' needs a `cron` expression"
		))
	})?;
	Ok(ScheduleSpec {
		cron,
		timezone,
		concurrency,
		missed_fires,
	})
}

fn unknown_key(label: &str, position: usize) -> ComposeError {
	job_error(format!(
		"schedule entry {position} of job '{label}' has an unknown key; the keys are `cron`, \
		 `timezone`, `concurrency` and `missed_fires`"
	))
}

fn text<'a>(label: &str, position: usize, key: &str, value: &'a Value) -> Result<&'a str> {
	value.as_str().ok_or_else(|| {
		job_error(format!(
			"`{key}` in schedule entry {position} of job '{label}' must be a string"
		))
	})
}

fn bad_choice(label: &str, position: usize, key: &str, allowed: &str) -> ComposeError {
	job_error(format!(
		"`{key}` in schedule entry {position} of job '{label}' must be {allowed}"
	))
}

/// A crontab expression: not blank, bounded, one printable line.
fn check_cron(label: &str, position: usize, cron: &str) -> Result<String> {
	let trimmed = cron.trim();
	if trimmed.is_empty() {
		return Err(job_error(format!(
			"schedule entry {position} of job '{label}' has an empty cron expression"
		)));
	}
	if trimmed.len() > MAX_CRON_BYTES {
		return Err(job_error(format!(
			"the cron expression in schedule entry {position} of job '{label}' is longer than \
			 {MAX_CRON_BYTES} bytes"
		)));
	}
	if trimmed.chars().any(char::is_control) {
		return Err(job_error(format!(
			"the cron expression in schedule entry {position} of job '{label}' contains a \
			 control character"
		)));
	}
	Ok(trimmed.to_string())
}

/// A time zone name: the characters IANA names use, bounded.
fn check_timezone(label: &str, position: usize, zone: &str) -> Result<String> {
	let valid = !zone.is_empty()
		&& zone.len() <= MAX_TIMEZONE_BYTES
		&& !zone.starts_with('/')
		&& !zone.contains("..")
		&& zone
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '+' | '-' | '.' | ':'));
	if valid {
		Ok(zone.to_string())
	} else {
		Err(job_error(format!(
			"`timezone` in schedule entry {position} of job '{label}' is not a time zone name \
			 (letters, digits and / _ + - . : up to {MAX_TIMEZONE_BYTES} bytes, no `..`, not absolute)"
		)))
	}
}

#[cfg(test)]
#[path = "triggers_tests.rs"]
mod tests;
