//! The `.timer` unit of one schedule entry.
//!
//! Every value written here is checked again at this point, whatever checks
//! came before: the calendar text, the time zone, and both unit names. The
//! function returns an error for a value that fails, and the caller reports the
//! schedule as not exported. Nothing in the file is interpolated from the
//! compose text without passing through those checks.

use super::on_calendar::is_safe_calendar;
use crate::quadlet::render::{unit_stem, Section};
use crate::quadlet::unit::owner_marker;
use crate::quadlet::QuadletUnit;

/// Longest time zone name accepted. IANA names stay well under this.
pub(super) const MAX_TIMEZONE_BYTES: usize = 64;
/// Longest unit file name systemd accepts, suffix included.
const MAX_UNIT_NAME_BYTES: usize = 255;

/// What one `.timer` file is made of.
pub(super) struct TimerSpec<'a> {
	/// Compose project name.
	pub project: &'a str,
	/// Name of the job in the compose file.
	pub job: &'a str,
	/// 1-based position of the schedule entry in `triggers.schedule`.
	pub index: usize,
	/// The `OnCalendar=` text from [`super::on_calendar::render`].
	pub calendar: &'a str,
	/// IANA time zone for the calendar, or `None` for the host's own zone.
	pub timezone: Option<&'a str>,
	/// `Persistent=`: run once on start when a fire was missed while off.
	pub persistent: bool,
}

/// The stem the job's `.container` unit has, and so the name of its service
/// without the `.service` suffix.
pub(super) fn job_stem(project: &str, job: &str) -> String {
	unit_stem(project, job)
}

/// Whether `name` is a plain systemd unit file name: ASCII letters, digits and
/// `_ . -`, no `..`, not starting with `.` or `-`, within the length limit.
/// No separator, NUL, `%`, `$`, `@` or whitespace can pass.
pub(super) fn is_safe_unit_name(name: &str) -> bool {
	!name.is_empty()
		&& name.len() <= MAX_UNIT_NAME_BYTES
		&& !name.starts_with(['.', '-'])
		&& !name.contains("..")
		&& name
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Whether `zone` is an IANA-style name: ASCII letters, digits and `_ + - /`,
/// no `..`, no empty segment, not starting with `-`, bounded.
pub(super) fn is_safe_timezone(zone: &str) -> bool {
	!zone.is_empty()
		&& zone.len() <= MAX_TIMEZONE_BYTES
		&& !zone.starts_with(['/', '-'])
		&& !zone.ends_with('/')
		&& !zone.contains("..")
		&& !zone.contains("//")
		&& zone
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '/'))
}

/// Build the `.timer` unit, or say which value failed its check.
pub(super) fn timer_unit(spec: &TimerSpec<'_>) -> Result<QuadletUnit, String> {
	if spec.project.chars().any(char::is_control) {
		return Err("the project name has a control character".into());
	}
	if !is_safe_calendar(spec.calendar) {
		return Err("the calendar text failed the final check".into());
	}
	if let Some(zone) = spec.timezone {
		if !is_safe_timezone(zone) {
			return Err(format!(
				"the time zone is not an IANA name (letters, digits and _ + - / up to \
				 {MAX_TIMEZONE_BYTES} bytes, no `..`)"
			));
		}
	}
	let stem = job_stem(spec.project, spec.job);
	let filename = format!("{stem}-{}.timer", spec.index);
	let service = format!("{stem}.service");
	for name in [&filename, &service] {
		if !is_safe_unit_name(name) {
			return Err("the unit name is not a plain systemd unit name".into());
		}
	}

	let mut unit = Section::new("Unit");
	unit.add(
		"Description",
		format!("{stem} schedule {} (podup)", spec.index),
	);
	let mut timer = Section::new("Timer");
	let on_calendar = match spec.timezone {
		Some(zone) => format!("{} {zone}", spec.calendar),
		None => spec.calendar.to_string(),
	};
	timer.add("OnCalendar", on_calendar);
	timer.add("Persistent", spec.persistent.to_string());
	timer.add("Unit", service);
	let mut install = Section::new("Install");
	install.add("WantedBy", "timers.target".to_string());

	let contents = format!(
		"{}{}\n{}\n{}",
		owner_marker(spec.project),
		unit.render(),
		timer.render(),
		install.render()
	);
	Ok(QuadletUnit { filename, contents })
}

#[cfg(test)]
#[path = "timer_tests.rs"]
mod tests;
