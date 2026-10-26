//! Profile filtering for jobs.
//!
//! A job with `profiles` is ignored unless one of them is active, exactly like
//! a service. A job never activates anything else: `config`, `audit`,
//! `generate quadlet` and `up` all filter through here or through
//! [`super::profiles::enabled_profile_services`], and a service behind an
//! inactive profile stays out of all of them even when a job depends on it.
//! Starting that service on behalf of the job is what `podup run` does, since
//! it names the job and so asks for its dependencies.
//!
//! A job whose dependency is filtered out is therefore ignored too, with a
//! warning that says why, because a job that kept a dependency nobody started
//! would be a file that fails validation.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::compose::dependencies::effective_depends_on;
use crate::compose::jobs::with_jobs_as_services;
use crate::compose::types::ComposeFile;
use crate::error::sanitize_name;

use super::profiles::service_in_profiles;

/// Drop the jobs excluded by the active profile set, in place, and then the
/// jobs left depending on a service outside `enabled_services` or on a
/// dropped job.
pub(super) fn retain_jobs(
	file: &mut ComposeFile,
	active: &HashSet<String>,
	targets: &[String],
	enabled_services: &HashSet<String>,
) {
	let named: HashSet<&str> = targets.iter().map(String::as_str).collect();
	let mut kept: HashSet<String> = file
		.jobs
		.iter()
		.filter(|(name, job)| {
			service_in_profiles(&job.service, active) || named.contains(name.as_str())
		})
		.map(|(name, _)| name.clone())
		.collect();

	// Work out once, per job, what it cannot do without, and push the loss
	// along the reverse edges with a queue: every job and every edge is looked
	// at a bounded number of times, however long a chain of jobs is.
	let everything = with_jobs_as_services(file);
	let mut needed_by: HashMap<String, Vec<String>> = HashMap::new();
	let mut lost: VecDeque<(String, String)> = VecDeque::new();
	for name in &kept {
		let Some(service) = everything.services.get(name) else {
			continue;
		};
		let deps = effective_depends_on(service, &everything.services);
		for dep in deps.service_names() {
			if !deps.required_for(&dep) {
				continue;
			}
			if file.jobs.contains_key(&dep) {
				if kept.contains(&dep) {
					needed_by.entry(dep).or_default().push(name.clone());
				} else {
					lost.push_back((name.clone(), dep));
				}
			} else if file.services.contains_key(&dep) && !enabled_services.contains(&dep) {
				lost.push_back((name.clone(), dep));
			}
		}
	}
	while let Some((name, dependency)) = lost.pop_front() {
		if !kept.remove(&name) {
			continue;
		}
		tracing::warn!(
			"job '{}' is ignored: it depends on '{}', which is behind an inactive profile; \
			 activate that profile or start the job with `podup run {}`",
			sanitize_name(&name),
			sanitize_name(&dependency),
			sanitize_name(&name)
		);
		for dependent in needed_by.get(&name).into_iter().flatten() {
			lost.push_back((dependent.clone(), name.clone()));
		}
	}
	file.jobs.retain(|name, _| kept.contains(name));
}

#[cfg(test)]
#[path = "profiles_jobs_tests.rs"]
mod tests;
