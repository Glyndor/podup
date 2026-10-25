//! The top-level `jobs:` section: containers that run to completion.
//!
//! The Compose Specification (v2.38) added a `jobs:` map next to `services:`.
//! A job takes every key a service takes, plus `triggers`. This module owns
//! what is specific to jobs and leaves the container machinery alone:
//!
//! - `parse` turns the raw YAML of each job into a typed
//!   [`Job`](crate::compose::types::Job), naming the job in every error
//!   (serde's own error carries no map key).
//! - `triggers` validates the `triggers:` block, `schedule` included.
//! - `validate` enforces the rules that tie jobs to services: one shared
//!   namespace, no service depending on a job, `extends` staying inside its
//!   own kind.
//! - `view` builds the [`ComposeFile`](crate::compose::types::ComposeFile)
//!   views the rest of podup already knows how to work on, so a job runs
//!   through the one-off machinery of a service instead of a second copy of it.
//! - `merge` combines a job declared in more than one `-f` file.
//! - `warn` says, once per job, what podup does not do for a job.
//!
//! `podup up` starts services only. A job runs when `podup run` names it, or
//! when a job that is being run depends on it.

mod merge;
mod parse;
mod triggers;
mod validate;
mod view;
mod warn;

pub(crate) use merge::merge_job;
pub(crate) use parse::deserialize_jobs;
pub(super) use parse::extract_jobs;
pub(super) use validate::{check_relations, check_structure};
pub(crate) use view::{
	job_file_for_run, job_services_view, store_job_services, with_jobs_as_services,
};
pub use warn::warn_unscheduled_jobs;
