//! Parse tests for features present in the type system but not previously covered.
//!
//! Split out of `tests/compose/parsing/parse/coverage/deploy_build.rs` so each file stays under
//! the repository line limit; this file holds the shared imports and the `mod`
//! lines, and the scenario files inherit the imports with `use super::*`.

use podup::compose::types::*;
use podup::parse_str;

// The top-level `configs:` section and the long-form service config reference.
mod configs;

// The `ipam` block of a network.
mod ipam;

// The top-level service `pids_limit` field.
mod pids_limit;

// The `deploy:` block: restart, update, rollback, placement and mode.
mod deploy;

// The long-form `build:` block.
mod build;
