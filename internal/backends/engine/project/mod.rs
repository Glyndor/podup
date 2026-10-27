//! Per-project handling: discovery of running projects, the per-project
//! label filter cache, naming helpers, profile filtering, replica resolution,
//! and the per-user advisory project lock with its private staging base.

pub(in crate::backends::engine) mod discovery;
pub use discovery::{list_projects, list_projects_filtered, LsOptions};
pub(in crate::backends::engine) mod label;
pub(in crate::backends::engine) mod lock;
pub use lock::{is_safe_project_name, ProjectLock};
pub(in crate::backends::engine) mod names;
pub(in crate::backends::engine) mod profiles;
mod profiles_jobs;
pub use profiles::{retain_active_profiles, retain_active_profiles_with_targets, retain_up_scope};
pub(in crate::backends::engine) mod replicas;
