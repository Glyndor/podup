//! The top-level `jobs:` section: parsed, but refused.
//!
//! The Compose Specification (v2.38) added a top-level `jobs:` map: containers
//! that run to completion, each with its own `triggers`, the container and
//! workload specification, `profiles` and `extends`. podup runs no job runner,
//! so a file that declares a job would start the project and leave the job
//! silently unrun, which reads as a job that succeeded. A file that declares a
//! job is therefore refused at parse time, before the project reaches the
//! engine.
//!
//! An empty `jobs:` map and an explicit `jobs: null` declare nothing: they are
//! accepted and ignored, exactly like a file without the key.

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

/// Refuse a compose file that declares top-level jobs.
///
/// The first job's name is part of the message: the operator has to know which
/// declaration to remove, and a file may declare several. The name is escaped
/// before it reaches the message, because it comes from the file and lands on
/// a terminal.
pub(super) fn reject_jobs(file: &ComposeFile) -> Result<()> {
	let Some(jobs) = file.jobs.as_ref() else {
		return Ok(());
	};
	let Some(first) = jobs.keys().next() else {
		return Ok(());
	};
	Err(ComposeError::Unsupported(format!(
		"jobs: the top-level jobs section is not supported by podup yet; remove it or run \
		 those containers with another tool (job '{}')",
		crate::error::sanitize_name(first)
	)))
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
