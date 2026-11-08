//! Report compose fields that are set but have no Quadlet mapping.
//!
//! The warnings are grouped by what they are about: the jobs a project
//! declares, the fields of a service, and the fields of a `build:` block.

mod build;
mod jobs;
mod service;

pub(super) use build::collect_build_warnings;
pub(super) use jobs::{jobs_not_exported, manual_only_jobs, timers_need_systemd_dir};
pub(super) use service::collect_warnings;

#[cfg(test)]
#[path = "../warnings_tests.rs"]
mod tests;
