//! The warnings about the jobs a project declares.

use indexmap::IndexMap;

/// The warning the autostart install gets for a project with jobs: it writes no
/// job units. Without it the install would exit 0 and say nothing about the
/// job, which reads as a job that systemd now runs on its schedule.
pub(crate) fn jobs_not_exported(
	jobs: &IndexMap<String, crate::compose::types::Job>,
) -> Option<String> {
	if jobs.is_empty() {
		return None;
	}
	Some(format!(
		"{} job(s) were not exported ({}): `autostart` installs services only; \
		 `podup generate quadlet` exports the schedule of a job as a systemd timer",
		jobs.len(),
		job_names(jobs.keys().map(String::as_str))
	))
}

/// The warning for jobs that have no `schedule`, so nothing can be exported
/// for them. One line for all of them, like the services' own summaries.
pub(crate) fn manual_only_jobs(names: &[&str]) -> Option<String> {
	if names.is_empty() {
		return None;
	}
	Some(format!(
		"{} job(s) have no schedule and were not exported ({}); run them with `podup run`",
		names.len(),
		job_names(names.iter().copied())
	))
}

/// The warning that goes with the `.timer` units: Quadlet only turns its own
/// unit types into services, so the timers have to be installed by hand.
pub(crate) fn timers_need_systemd_dir(count: usize) -> String {
	format!(
		"{count} .timer unit(s) were written for job schedules; Quadlet does not read .timer \
		 files, so copy them to the systemd unit directory (for example ~/.config/systemd/user), \
		 run `systemctl --user daemon-reload` and enable each one with `systemctl --user enable \
		 --now NAME.timer`; the .container unit of the job stays where the other Quadlet units are"
	)
}

/// Up to five names, sanitized for the terminal, then "and N more".
fn job_names<'a>(names: impl ExactSizeIterator<Item = &'a str>) -> String {
	const SHOWN: usize = 5;
	let total = names.len();
	let mut shown: Vec<String> = names
		.take(SHOWN)
		.map(|name| crate::error::sanitize_name(name).into_owned())
		.collect();
	if total > SHOWN {
		shown.push(format!("and {} more", total - SHOWN));
	}
	shown.join(", ")
}
