//! Saying "job" where a message about a service would say "service".
//!
//! A job runs through the checks and warnings written for services, and those
//! name their subject as `service 'x'`. When the subject is a job the wording
//! is corrected at the edge, in one place, instead of threading a noun through
//! every message.

use crate::compose::types::ComposeFile;
use crate::error::{sanitize_name, ComposeError};

use super::parse::dependency_error;

/// Relabel a warning that begins with `service 'x'` or `service "x"` when `x`
/// (or the job an init step `x/pre_start[i]` belongs to) is a job of `file`.
pub(crate) fn relabel(file: &ComposeFile, message: String) -> String {
	for quote in ['\'', '"'] {
		let Some(rest) = message.strip_prefix(&format!("service {quote}")) else {
			continue;
		};
		let name = rest.split(quote).next().unwrap_or_default();
		let base = name.split('/').next().unwrap_or(name);
		if file.jobs.contains_key(base) {
			return format!("job {quote}{rest}");
		}
	}
	message
}

/// Relabel an error raised while checking the job `name` as a service.
pub(crate) fn relabel_error(name: &str, error: ComposeError) -> ComposeError {
	let swap = |text: String| {
		text.replace(&format!("service '{name}'"), &format!("job '{name}'"))
			.replacen("service name", "job name", 1)
	};
	match error {
		ComposeError::Unsupported(text) => ComposeError::Unsupported(swap(text)),
		ComposeError::InvalidPort(text) => ComposeError::InvalidPort(swap(text)),
		ComposeError::NoImageOrBuild(_) => ComposeError::Job(format!(
			"job '{}' has neither an image nor a build",
			sanitize_name(name)
		)),
		other => other,
	}
}

/// A job whose required `depends_on` names neither a service nor a job.
pub(crate) fn check_job_dependencies(file: &ComposeFile) -> Result<(), ComposeError> {
	let everything = super::with_jobs_as_services(file);
	for (name, job) in &file.jobs {
		let deps =
			crate::compose::dependencies::effective_depends_on(&job.service, &everything.services);
		for dep in deps.service_names() {
			if deps.required_for(&dep) && !everything.services.contains_key(&dep) {
				return Err(dependency_error(name, &dep));
			}
		}
	}
	Ok(())
}

#[cfg(test)]
#[path = "label_tests.rs"]
mod tests;
