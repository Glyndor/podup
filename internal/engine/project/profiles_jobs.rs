//! Profile filtering for jobs.
//!
//! A job with `profiles` is ignored unless one of them is active, exactly like
//! a service. What a retained job needs comes with it: a profiled service it
//! depends on is kept, and so is a profiled job. That activation is one-way.
//! It never feeds back into [`super::profiles::enabled_profile_services`],
//! which decides what `up` starts, so a service that only a job needs stays
//! stopped when the stack comes up.

use std::collections::HashSet;

use crate::compose::dependencies::effective_depends_on;
use crate::compose::jobs::with_jobs_as_services;
use crate::compose::types::ComposeFile;

use super::profiles::service_in_profiles;

/// Drop the jobs excluded by the active profile set, in place, and add to
/// `enabled_services` the services the surviving jobs depend on.
pub(super) fn retain_jobs(
	file: &mut ComposeFile,
	active: &HashSet<String>,
	targets: &[String],
	enabled_services: &mut HashSet<String>,
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

	// Walk what the kept jobs depend on, through jobs and services alike.
	let everything = with_jobs_as_services(file);
	let mut stack: Vec<String> = kept.iter().cloned().collect();
	let mut seen: HashSet<String> = kept.clone();
	while let Some(name) = stack.pop() {
		let Some(service) = everything.services.get(&name) else {
			continue;
		};
		for dep in effective_depends_on(service, &everything.services).service_names() {
			if !seen.insert(dep.clone()) {
				continue;
			}
			if file.jobs.contains_key(&dep) {
				kept.insert(dep.clone());
			} else if file.services.contains_key(&dep) {
				enabled_services.insert(dep.clone());
			} else {
				continue;
			}
			stack.push(dep);
		}
	}
	file.jobs.retain(|name, _| kept.contains(name));
}

#[cfg(test)]
#[path = "profiles_jobs_tests.rs"]
mod tests;
