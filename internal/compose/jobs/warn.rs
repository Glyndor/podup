//! What podup says about a job it does not act on by itself.
//!
//! A `schedule` trigger is parsed and validated, and podup never fires it.
//! Accepting it quietly would read as a job that runs on time, so each job
//! with a schedule is reported once, however many entries it lists.

use crate::compose::types::{ComposeFile, Job};
use crate::error::sanitize_name;

/// One sentence per job whose triggers podup cannot satisfy by itself.
///
/// A job with a schedule and no `manual: false` can still be launched by hand,
/// and the sentence says so. A job with `manual: false` has no trigger podup
/// can pull, scheduled or not, and the sentence says that instead.
pub(crate) fn unscheduled_messages<'a>(
	jobs: impl IntoIterator<Item = (&'a String, &'a Job)>,
) -> Vec<String> {
	jobs.into_iter()
		.filter_map(|(name, job)| message_for(name, job))
		.collect()
}

fn message_for(name: &str, job: &Job) -> Option<String> {
	let name = sanitize_name(name);
	let entries = job.triggers.schedule.len();
	let count = if entries == 1 {
		"1 entry".to_string()
	} else {
		format!("{entries} entries")
	};
	let manual = job.triggers.allows_manual();
	match (entries, manual) {
		(0, true) => None,
		(0, false) => Some(format!(
			"job '{name}' has `manual: false` and no schedule, so nothing can start it"
		)),
		(_, true) => Some(format!(
			"job '{name}' has a schedule trigger ({count}) that podup does not fire by \
			 itself, and `podup generate quadlet` does not export jobs yet; the job runs only \
			 when you start it with `podup run {name}`"
		)),
		(_, false) => Some(format!(
			"job '{name}' has a schedule trigger ({count}) and `manual: false`; podup \
			 does not fire schedules by itself and `podup generate quadlet` does not export \
			 jobs yet, so nothing starts this job"
		)),
	}
}

/// Emit the warnings for `jobs`, one `warn` event each.
pub(crate) fn warn_unscheduled<'a>(jobs: impl IntoIterator<Item = (&'a String, &'a Job)>) {
	for message in unscheduled_messages(jobs) {
		tracing::warn!("{message}");
	}
}

/// Warn, once per job, about the jobs of `file` that podup does not trigger
/// by itself. Called by `up` and `config` on the profile-filtered file, so a
/// job hidden behind an inactive profile stays quiet.
pub fn warn_unscheduled_jobs(file: &ComposeFile) {
	warn_unscheduled(&file.jobs);
}

#[cfg(test)]
#[path = "warn_tests.rs"]
mod tests;
