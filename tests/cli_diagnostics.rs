//! CLI-level checks for diagnostic surfacing: forward-compat warnings must be
//! visible with no `RUST_LOG` set, must go to stderr, and must carry the
//! `podup:` program prefix, while stdout stays a clean YAML pipe for `config`.
//!
//! This crate root holds the helpers every scenario file inherits through
//! `use super::*`, plus one `mod` line per scenario. The rows themselves live
//! in the files declared below.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Write `contents` as a compose file in a temp dir of its own.
///
/// The directory must be unique **per call**, not per process: cargo runs the
/// tests in one binary as threads, so a path keyed on the pid is shared by every
/// test in this file. Two tests writing the same compose file then race:
/// `fs::write` truncates before it writes, so a concurrent reader can see an
/// empty file and fail validation. Returning the [`TempDir`] hands the caller
/// the directory's lifetime: hold it for the length of the test, and it is
/// removed on drop rather than left in /tmp forever.
fn compose_file(name: &str, contents: &str) -> (TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("create temp dir");
	let path = dir.path().join(name);
	fs::write(&path, contents).expect("write compose file");
	(dir, path)
}

// An unsupported key in the compose file drives the forward-compat diagnostic.
#[path = "cli_diagnostics/forward_compat_warning.rs"]
mod forward_compat_warning;

// The output surfaces of the `config` subcommand: `--services`,
// `--format json` and `-q`.
#[path = "cli_diagnostics/config_surfaces.rs"]
mod config_surfaces;

// The `completions` subcommand writes its script to stdout.
#[path = "cli_diagnostics/completions.rs"]
mod completions;

// `--no-interpolate` prints the file without evaluating placeholders.
#[path = "cli_diagnostics/no_interpolate.rs"]
mod no_interpolate;

// Mutually exclusive flag pairs are rejected at parse time.
#[path = "cli_diagnostics/flag_conflicts.rs"]
mod flag_conflicts;

// Unset interpolation variables warn exactly once each, on `config` and on
// `audit`.
#[path = "cli_diagnostics/unset_variable_warnings.rs"]
mod unset_variable_warnings;

// A compose-only global flag passed to `update` is a usage error.
#[path = "cli_diagnostics/global_flag_misuse.rs"]
mod global_flag_misuse;

// The 16 MiB read cap, over stdin and over a file.
#[path = "cli_diagnostics/read_cap.rs"]
mod read_cap;
