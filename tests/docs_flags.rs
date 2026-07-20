//! `docs/commands.md` must document every flag the CLI actually accepts.
//!
//! The command reference was written per command by hand and drifted every time
//! a flag landed — #1084 counted fourteen commands with missing flags and one
//! precedence rule that was simply false. Fixing the text once only resets the
//! clock; this test is what stops it recurring, by diffing `--help` against the
//! document rather than trusting either.
//!
//! It asserts one direction only: every flag clap exposes appears in the docs.
//! The reverse (documented flags that no longer exist) is a different failure
//! and a different fix, and folding both into one assertion would make the
//! message ambiguous.

use std::collections::BTreeSet;
use std::process::Command;

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Long flags clap prints in a `--help` body, e.g. `--no-cache`.
///
/// Only long forms are collected: a short form is always printed beside its long
/// one, so requiring the long spelling covers both without needing to model
/// clap's `-x, --xyz` layout.
fn flags_in_help(help: &str) -> BTreeSet<String> {
	let mut out = BTreeSet::new();
	for line in help.lines() {
		let trimmed = line.trim_start();
		// Flag lines start with the short form or the long one; anything else is
		// prose, a usage line, or a subcommand listing.
		if !trimmed.starts_with('-') {
			continue;
		}
		for token in trimmed.split_whitespace() {
			let token = token.trim_end_matches([',', '.']);
			if let Some(name) = token.strip_prefix("--") {
				let name = name
					.split(['=', '<', '['])
					.next()
					.unwrap_or_default()
					.trim_end_matches('>');
				if !name.is_empty() && name != "help" && name != "version" {
					out.insert(format!("--{name}"));
				}
			}
		}
	}
	out
}

/// The subcommands whose flag tables the reference documents. Aliases are
/// excluded: they resolve to the same command and share its table.
const COMMANDS: [&str; 29] = [
	"up", "down", "create", "start", "stop", "restart", "build", "ps", "ls", "logs", "events",
	"top", "stats", "port", "images", "volumes", "run", "exec", "cp", "attach", "kill", "rm",
	"wait", "scale", "commit", "export", "pull", "push", "config",
];

#[test]
fn every_cli_flag_is_documented() {
	let docs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/commands.md"))
		.expect("read docs/commands.md");

	let mut missing: Vec<String> = Vec::new();
	for cmd in COMMANDS {
		let out = Command::new(bin())
			.args([cmd, "--help"])
			.output()
			.unwrap_or_else(|e| panic!("run {cmd} --help: {e}"));
		assert!(out.status.success(), "`{cmd} --help` failed");
		let help = String::from_utf8_lossy(&out.stdout);
		for flag in flags_in_help(&help) {
			if !docs.contains(&flag) {
				missing.push(format!("{cmd}: {flag}"));
			}
		}
	}

	assert!(
		missing.is_empty(),
		"flags accepted by the CLI but absent from docs/commands.md:\n  {}",
		missing.join("\n  ")
	);
}

/// The global options table is written separately from the per-command ones, so
/// it drifts separately too.
#[test]
fn every_global_flag_is_documented() {
	let docs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/commands.md"))
		.expect("read docs/commands.md");
	let out = Command::new(bin())
		.arg("--help")
		.output()
		.expect("run --help");
	let help = String::from_utf8_lossy(&out.stdout);

	let missing: Vec<String> = flags_in_help(&help)
		.into_iter()
		.filter(|f| !docs.contains(f))
		.collect();
	assert!(
		missing.is_empty(),
		"global flags absent from docs/commands.md:\n  {}",
		missing.join("\n  ")
	);
}
