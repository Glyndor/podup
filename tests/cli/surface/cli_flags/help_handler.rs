//! The `help` handler is tolerant of extra arguments, and a missing
//! subcommand exits non-zero while printing the usage that tells the operator
//! what to type.
//!
//! Split out from `tests/cli/surface/cli_flags.rs`; the helper `bin` is inherited via
//! `use super::*`.

use super::*;

#[test]
fn help_subcommand_is_tolerant() {
	for args in [
		&["help", "up", "down"][..],
		&["help", "-h"][..],
		&["help", "--help"][..],
		&["help", "--", "up"][..],
		&["help"][..],
	] {
		let out = Command::new(bin())
			.args(args)
			.output()
			.expect("run help variant");
		assert!(
			out.status.success(),
			"`{args:?}` should exit 0; got {:?}\n{}",
			out.status.code(),
			String::from_utf8_lossy(&out.stderr)
		);
	}
	// `help up down` shows up's help.
	let out = Command::new(bin())
		.args(["help", "up", "down"])
		.output()
		.unwrap();
	assert!(String::from_utf8_lossy(&out.stdout).contains("Create and start"));
}

#[test]
fn missing_subcommand_exits_non_zero() {
	let none = Command::new(bin())
		.output()
		.expect("run podup with no args");
	assert!(!none.status.success(), "no args must exit non-zero");
	// Exiting non-zero with nothing on stderr would satisfy the check above and
	// leave the user with no idea what to type. clap prints usage; assert it.
	let none_err = String::from_utf8_lossy(&none.stderr);
	assert!(
		none_err.contains("Usage: podup"),
		"running with no arguments printed no usage: {none_err:?}"
	);
	let gen = Command::new(bin())
		.arg("generate")
		.output()
		.expect("run generate");
	assert!(
		!gen.status.success(),
		"generate with no subcommand must exit non-zero"
	);
	// The group's own usage, not the root's. Until #1293 this printed
	// `Usage: podup [OPTIONS] <COMMAND>` and listed every top-level command,
	// withholding the one thing the user needed, what `generate` accepts,
	// which its own `--help` had been answering correctly all along.
	let gen_err = String::from_utf8_lossy(&gen.stderr);
	assert!(
		gen_err.contains("Usage: podup generate"),
		"generate with no subcommand did not print generate's own usage: {gen_err:?}"
	);
	assert!(
		gen_err.contains("quadlet"),
		"generate's usage did not list the subcommand it accepts: {gen_err:?}"
	);
}

/// Bare `podup` is the first screen anyone sees after installing, and it was
/// the one help path that rendered without colour, while every other screen had it,
/// so podup looked like a tool that does not colourise at all.
///
/// This checks the piped side, which is what a test harness can observe: the
/// help must reach **stderr** (it is a usage error, not output), carry no
/// escape codes when stderr is not a terminal, and stay on exit 2. The coloured
/// side is verified by running it under a pty, which no `Command::output()`
/// test can do: 50 escapes against 0 before.
#[test]
fn bare_podup_prints_help_to_stderr_and_stays_plain_when_piped() {
	let out = Command::new(bin())
		.output()
		.expect("run podup with no args");
	let stderr = String::from_utf8_lossy(&out.stderr);
	let stdout = String::from_utf8_lossy(&out.stdout);

	assert_eq!(out.status.code(), Some(2), "no args is a usage error");
	assert!(
		stderr.contains("Usage:") && stderr.contains("Commands:"),
		"the help belongs on stderr: {stderr:?}"
	);
	assert!(
		stdout.is_empty(),
		"stdout must stay clean for scripts: {stdout:?}"
	);
	assert!(
		!stderr.contains('\u{1b}'),
		"a pipe must receive no escape codes: {stderr:?}"
	);
}

/// The same screen, whether or not the caller's environment happens to set one
/// of podup's env-backed globals.
///
/// `arg_required_else_help` only fires when there are NO arguments at all, and
/// an env-sourced one counts, so with `COMPOSE_PROJECT_NAME` or
/// `PODMAN_SOCKET` exported, which is the normal state of a real deployment,
/// bare `podup` printed a wall of forty-five subcommand names instead of its
/// help. This is how CI found it: the runner had one of them set and my first
/// version of the test above failed there while passing here.
#[test]
fn bare_podup_prints_the_same_help_with_env_globals_set() {
	for (key, value) in [
		("COMPOSE_PROJECT_NAME", "someproject"),
		("PODMAN_SOCKET", "/tmp/nonexistent.sock"),
		("COMPOSE_PROFILES", "dev"),
	] {
		let out = Command::new(bin())
			.env(key, value)
			.output()
			.unwrap_or_else(|e| panic!("run podup with {key} set: {e}"));
		let stderr = String::from_utf8_lossy(&out.stderr);
		assert_eq!(out.status.code(), Some(2), "with {key} set");
		assert!(
			stderr.contains("Commands:"),
			"{key} set must not replace the help with a subcommand list: {stderr:?}"
		);
	}
}
