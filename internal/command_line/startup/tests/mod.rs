//! Startup-time tests for the `startup` module. Pulled into a separate file so
//! the production `mod.rs` stays under the 500-line file cap; the test surface
//! has grown with the help-walker and the `format_clap_error` helper, and a
//! few hundred lines of `#[test]` blocks are not where the `parse_cli`
//! implementation belongs.
//!
//! Every test of the folder lives here, one file per subject, so the folder
//! holds one convention instead of two.

use super::config_normalize::{
	anchor_service_bind_sources, block_scalar_parent, looks_like_yaml11_bool,
	quote_yaml11_booleans, resolve_bind_sources, rewrite_short_bind,
};
use super::config_render::{
	inject_resolved_network_names, prune_json_nulls, prune_yaml_nulls, render_config,
	render_config_to, service_config_hash, ConfigOutput,
};
use super::*;
use crate::cli::{ConfigArgs, ConfigFormat, EventsArgs, PsArgs, UpArgs};
use podup::compose::types::VolumeMount;
use std::path::Path;

mod apparmor_hint;
mod bridge;
mod config_normalize;
mod config_render;
mod control_escape;
mod exit_status;
mod help_for_tests;
mod resolve;

#[cfg(test)]
mod render_help_tests;

#[cfg(test)]
mod startup_tests;
