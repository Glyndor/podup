//! Compose file parsing, `extends:` resolution, `include:` merging, and
//! topological service ordering.

pub mod types;

pub(crate) mod diagnostics;
pub(crate) mod jobs;
mod merging;
mod resolve;

use merging::{anchor, extends, include, merge, nested_resets, tags};
pub(crate) use resolve::dependencies;
pub use resolve::pre_start;
use resolve::{order, validate};

mod parsing;
mod project_name;

pub use diagnostics::{ShowClientAddressWarningGuard, SuppressPortExposureGuard};
pub use jobs::warn_unscheduled_jobs;
pub use order::{resolve_levels, resolve_order};
#[cfg(test)]
use parsing::merge_override;
#[cfg(test)]
pub(crate) use parsing::normalize_default_network;
pub(crate) use parsing::parse_file_inner_with_env;
use parsing::{interpolated_yaml_text, is_stdin};
pub use parsing::{
	parse_file_with_env_files, parse_file_with_env_files_interp, parse_files_with_env_files,
	parse_files_with_env_files_interp, parse_files_with_env_files_interp_in,
	parse_files_with_project_in, parse_str, parse_str_raw,
};
pub use project_name::{resolve_base_dir, resolve_project_name, sanitize_project_name};
#[cfg(test)]
use types::ComposeFile;
pub use validate::validate_config;

#[cfg(test)]
mod diagnostics_budget_tests;
#[cfg(test)]
mod entry_reset_tests;
#[cfg(test)]
mod nested_reset_tests;
#[cfg(test)]
mod pull_refresh_tests;
#[cfg(test)]
mod toplevel_merge_tests;

#[cfg(test)]
mod pre_start_tests;
#[cfg(test)]
mod tag_interpolation_tests;
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
mod empty_networks_tests;
