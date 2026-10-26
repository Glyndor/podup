//! CLI-surface checks for the flag-parity sweep: interspersed flags after a
//! service positional, value validation, conflicting-flag rejection, the
//! tolerant `help` handler, non-zero exits for missing subcommands, and the new
//! per-command flags. These exercise the built binary's argument parsing and
//! exit codes; none of them contact Podman (they either stop at `--help`, fail
//! parsing, or reach runtime against a nonexistent socket).
//!
//! This crate root holds the helpers every scenario file inherits through
//! `use super::*`, plus one `mod` line per scenario. The rows themselves live
//! in the files declared below.

use std::fs;
use std::process::{Command, Output};

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// True when clap rejected the arguments themselves (exit 2 with the `error:`/
/// `Usage:` banner) rather than parsing and failing later at runtime.
fn is_clap_usage_error(out: &Output) -> bool {
	let stderr = String::from_utf8_lossy(&out.stderr);
	out.status.code() == Some(2) && stderr.contains("error:") && stderr.contains("Usage:")
}

/// True when clap rejected a value (exit 2 with an `error:` line). Value-parser
/// errors print the offending value and a `--help` hint but no `Usage:` banner.
fn is_clap_value_error(out: &Output) -> bool {
	let stderr = String::from_utf8_lossy(&out.stderr);
	out.status.code() == Some(2) && stderr.contains("error:")
}

/// Run podup against a nonexistent socket so a parsed command reaches runtime
/// (and fails connecting) instead of being rejected by clap.
fn run_offline(args: &[&str]) -> Output {
	let mut full = vec!["--socket", "/nonexistent.sock"];
	full.extend_from_slice(args);
	Command::new(bin())
		.args(&full)
		.output()
		.expect("run podup offline")
}

fn help_of(cmd: &str) -> String {
	let out = Command::new(bin())
		.args([cmd, "--help"])
		.output()
		.expect("run <cmd> --help");
	assert!(out.status.success(), "`{cmd} --help` failed");
	String::from_utf8_lossy(&out.stdout).into_owned()
}

// --- #717: flags after a service positional are no longer swallowed ----------
// and #863/#862: the positionals other commands take.

#[path = "cli_flags/positionals.rs"]
mod positionals;

// --- #747: invalid pull policy rejected at parse time ------------------------
// #842: negative --timeout rejected in both spellings
// --sig-proxy must accept docker's bare spelling.

#[path = "cli_flags/value_validation.rs"]
mod value_validation;

// --- #840/#843: up --wait-timeout, and it requires --wait -------------------
// #841: conflicting recreate/build flags are rejected
// #861: deprecated --json conflicts with an explicit --format
// ps --services conflicts with --quiet and --format

#[path = "cli_flags/conflicting_flags.rs"]
mod conflicting_flags;

// --- #844: create --pull / --no-deps ----------------------------------------
// #845/#847/#848/#849/#850/#851/#854: new per-command flags in help
// #855: --ansi help mentions the NO_COLOR override

#[path = "cli_flags/help_text.rs"]
mod help_text;

// --- #857: help tolerates extra args, -h/--help, and -- ---------------------
// #858/#859: missing subcommand exits non-zero

#[path = "cli_flags/help_handler.rs"]
mod help_handler;

// --- #860: --project-directory must exist -----------------------------------
// #750: DOCKER_HOST with a remote scheme is rejected (not ignored)
// #1687: a named compose file that is missing fails the command

#[path = "cli_flags/global_configuration.rs"]
mod global_configuration;

// --- #856: config list projections render the expected lines ----------------

#[path = "cli_flags/config_projections.rs"]
mod config_projections;

// --- Machine output never carries colour, even when forced ------------------

#[path = "cli_flags/machine_output.rs"]
mod machine_output;
