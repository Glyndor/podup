//! Startup-time tests for the `startup` module. Pulled into a separate file so
//! the production `mod.rs` stays under the 500-line file cap; the test surface
//! has grown with the help-walker and the `format_clap_error` helper, and a
//! few hundred lines of `#[test]` blocks are not where the `parse_cli`
//! implementation belongs.

use super::*;
use crate::cli::{ConfigArgs, EventsArgs, PsArgs, UpArgs};

mod help_for_tests;

#[cfg(test)]
mod render_help_tests;

#[cfg(test)]
mod startup_tests;
