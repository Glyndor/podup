//! Per-project handling: discovery of running projects, the per-project
//! label filter cache, naming helpers, profile filtering, replica resolution,
//! and the per-user advisory project lock with its private staging base.

pub(in crate::engine) mod discovery;
pub use discovery::{list_projects, list_projects_filtered, LsOptions};
pub(in crate::engine) mod label;
pub(in crate::engine) mod lock;
pub use lock::{is_safe_project_name, ProjectLock};
pub(in crate::engine) mod names;
pub(in crate::engine) mod profiles;
pub use profiles::{retain_active_profiles, retain_active_profiles_with_targets};
pub(in crate::engine) mod replicas;
