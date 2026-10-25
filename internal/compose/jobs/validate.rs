//! The rules that tie jobs to services.
//!
//! Two passes, split by what each can see. [`check_structure`] runs before
//! `extends` is resolved and needs only the names: services and jobs share one
//! namespace, and `extends` may not cross from one kind to the other.
//! [`check_relations`] runs after `extends` is resolved, when every
//! `depends_on` is final: a service cannot depend on a job.

use super::parse::job_error;
use super::view::with_jobs_as_services;
use crate::compose::dependencies::effective_depends_on;
use crate::compose::types::ComposeFile;
use crate::error::{sanitize_name, Result};

/// Names and `extends` targets, before `extends` is resolved.
pub(in crate::compose) fn check_structure(file: &ComposeFile) -> Result<()> {
	for name in file.jobs.keys() {
		if file.services.contains_key(name) {
			return Err(job_error(format!(
				"the name '{}' is declared as a service and as a job; services and jobs share \
				 one namespace, so rename one of them",
				sanitize_name(name)
			)));
		}
	}
	for (name, job) in &file.jobs {
		let Some(target) = local_extends(job.service.extends.as_ref()) else {
			continue;
		};
		if file.services.contains_key(target) {
			return Err(job_error(format!(
				"job '{}' extends '{}', which is a service; a job can only extend another job",
				sanitize_name(name),
				sanitize_name(target)
			)));
		}
	}
	for (name, service) in &file.services {
		let Some(target) = local_extends(service.extends.as_ref()) else {
			continue;
		};
		if file.jobs.contains_key(target) {
			return Err(job_error(format!(
				"service '{}' extends '{}', which is a job; a service can only extend another \
				 service",
				sanitize_name(name),
				sanitize_name(target)
			)));
		}
	}
	Ok(())
}

/// The `extends` target that lives in this same file, if there is one.
fn local_extends(extends: Option<&crate::compose::types::ExtendsConfig>) -> Option<&str> {
	extends
		.filter(|extends| extends.file().is_none())
		.map(|extends| extends.service())
}

/// Every job declares at least one trigger, once all the `-f` files are merged.
pub(in crate::compose) fn check_triggers_declared(file: &ComposeFile) -> Result<()> {
	for (name, job) in &file.jobs {
		if job.triggers.manual.is_none() && job.triggers.schedule.is_empty() {
			return Err(job_error(format!(
				"job '{}' declares no triggers; add `triggers:` with `manual: true` or a \
				 `schedule:` list",
				sanitize_name(name)
			)));
		}
	}
	Ok(())
}

/// A service must not depend on a job, whether it says so in `depends_on` or
/// reaches the job through `volumes_from`, `links` or `service:` namespaces.
pub(in crate::compose) fn check_relations(file: &ComposeFile) -> Result<()> {
	if file.jobs.is_empty() {
		return Ok(());
	}
	let everything = with_jobs_as_services(file).services;
	for (name, service) in &file.services {
		let deps = effective_depends_on(service, &everything);
		if let Some(job) = deps
			.service_names()
			.into_iter()
			.find(|dep| file.jobs.contains_key(dep))
		{
			return Err(job_error(format!(
				"service '{}' depends on job '{}'; a service cannot depend on a job, only a job \
				 can depend on services and other jobs",
				sanitize_name(name),
				sanitize_name(&job)
			)));
		}
	}
	Ok(())
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
