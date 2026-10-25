//! Views of a compose file that treat jobs as services.
//!
//! A job is a [`Service`] plus triggers, so everything podup can do to a
//! service (resolve `extends`, order `depends_on`, create and start a
//! container, run the audit checks) can be done to a job by handing that code
//! a file in which the job sits under `services`. The views below build that
//! file. They are the reason jobs need no second copy of the container
//! machinery, and the reason `up` can stay unaware of jobs: the views are
//! only ever built by the commands that mean to act on a job.

use std::collections::HashSet;

use indexmap::IndexMap;

use super::parse::job_error;
use crate::compose::dependencies::effective_depends_on;
use crate::compose::types::{ComposeFile, Service};
use crate::error::{sanitize_name, ComposeError, Result};

impl ComposeFile {
	/// Every service and every job's service body, for passes that must treat
	/// both the same way (anchoring paths, folding `env_file`, normalizing
	/// networks).
	pub fn services_and_jobs_mut(&mut self) -> impl Iterator<Item = &mut Service> {
		self.services
			.values_mut()
			.chain(self.jobs.values_mut().map(|job| &mut job.service))
	}
}

/// A file whose `services` are the jobs of `file`, and nothing else. Used to
/// resolve `extends` inside the job namespace.
pub(crate) fn job_services_view(file: &ComposeFile) -> ComposeFile {
	ComposeFile {
		services: file
			.jobs
			.iter()
			.map(|(name, job)| (name.clone(), job.service.clone()))
			.collect::<IndexMap<_, _>>(),
		..ComposeFile::default()
	}
}

/// Write the services of a [`job_services_view`] back into the jobs they
/// came from, after a pass that changed them (`extends` resolution).
pub(crate) fn store_job_services(file: &mut ComposeFile, view: ComposeFile) {
	for (name, service) in view.services {
		if let Some(job) = file.jobs.get_mut(&name) {
			job.service = service;
		}
	}
}

/// A copy of `file` with every job listed under `services` and no `jobs`.
///
/// The names cannot collide: services and jobs share one namespace, which
/// `check_structure` enforces when the file is parsed.
pub(crate) fn with_jobs_as_services(file: &ComposeFile) -> ComposeFile {
	let mut view = file.clone();
	for (name, job) in std::mem::take(&mut view.jobs) {
		view.services.entry(name).or_insert(job.service);
	}
	view
}

/// The file `podup run <job>` works on.
pub(crate) struct RunView {
	/// All services, plus the job and the jobs it depends on, as services.
	pub file: ComposeFile,
	/// The jobs that will run, the requested one first.
	pub jobs: Vec<String>,
}

/// Build the view for running the job `target`.
///
/// Refuses a job whose `triggers.manual` is `false`. Only the jobs `target`
/// reaches through `depends_on` are included, so a job nobody asked for is
/// neither validated nor pulled.
pub(crate) fn run_view(file: &ComposeFile, target: &str) -> Result<RunView> {
	let job = file
		.jobs
		.get(target)
		.ok_or_else(|| ComposeError::ServiceNotFound(target.to_string()))?;
	if !job.triggers.allows_manual() {
		return Err(job_error(format!(
			"job '{}' sets `triggers.manual` to false, so `podup run` cannot launch it",
			sanitize_name(target)
		)));
	}
	let mut view = with_jobs_as_services(file);
	let mut reached: Vec<String> = vec![target.to_string()];
	let mut seen: HashSet<String> = reached.iter().cloned().collect();
	let mut next = 0;
	while let Some(name) = reached.get(next).cloned() {
		next += 1;
		let Some(service) = view.services.get(&name) else {
			continue;
		};
		for dep in effective_depends_on(service, &view.services).service_names() {
			if file.jobs.contains_key(&dep) && seen.insert(dep.clone()) {
				reached.push(dep);
			}
		}
	}
	view.services
		.retain(|name, _| !file.jobs.contains_key(name) || seen.contains(name));
	Ok(RunView {
		file: view,
		jobs: reached,
	})
}

/// For `run NAME`: the file to run on when `NAME` is a job, `None` when it
/// is a service (or unknown) and `file` is used as it is. Reports the
/// schedule of each job about to run.
pub(crate) fn job_file_for_run(file: &ComposeFile, name: &str) -> Result<Option<ComposeFile>> {
	if file.services.contains_key(name) || !file.jobs.contains_key(name) {
		return Ok(None);
	}
	let view = run_view(file, name)?;
	super::warn::warn_unscheduled(view.jobs.iter().filter_map(|n| file.jobs.get_key_value(n)));
	Ok(Some(view.file))
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
