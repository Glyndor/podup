//! `extends`: how a base service's keys reach the service that extends it,
//! within one file and across files.
//!
//! Split out of `tests/compose/parsing/parse/reuse/extends/mod.rs` so each file stays under the
//! repository line limit; this file holds the shared imports and the `mod`
//! lines, and the scenario files inherit the imports with `use super::*`.

use podup::{parse_files_with_env_files, parse_str};

// Merging and overriding inside one file.
mod same_file;

// The three rejections a same-file `extends` can earn.
mod same_file_errors;

// Resolving a base service out of another file, and anchoring its relative
// paths to that file's directory.
mod external_file;

// The rejections an external `extends` can earn.
mod external_file_errors;

// The chain-depth guard.
mod depth_limit;

// The same-file cases again, parsed from disk, so the on-disk resolver runs.
mod from_disk;
