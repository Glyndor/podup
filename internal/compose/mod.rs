//! Compose file parsing, `extends:` resolution, `include:` merging, and
//! topological service ordering.

pub mod types;

mod anchor;
pub(crate) mod dependencies;
pub(crate) mod diagnostics;
mod extends;
mod include;
mod jobs;
mod merge;
mod order;
mod tags;
mod validate;

mod parsing;

pub use diagnostics::{ShowClientAddressWarningGuard, SuppressPortExposureGuard};
pub use order::{resolve_levels, resolve_order};
#[cfg(test)]
use parsing::merge_override;
#[cfg(test)]
pub(crate) use parsing::normalize_default_network;
pub(crate) use parsing::parse_file_inner_with_env;
use parsing::{interpolated_yaml_text, is_stdin};
pub use parsing::{
	parse_file_with_env_files, parse_file_with_env_files_interp, parse_files_with_env_files,
	parse_files_with_env_files_interp, parse_str, parse_str_raw,
};
#[cfg(test)]
use types::ComposeFile;
pub use validate::validate_config;

#[cfg(test)]
mod pull_refresh_tests;
#[cfg(test)]
mod tag_interpolation_tests;
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
