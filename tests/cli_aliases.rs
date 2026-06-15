//! CLI surface checks for the convenience command aliases and the blank-line
//! framing around `--help`/`--version` output. Aliases are hidden (they must
//! resolve when typed but stay out of the top-level help listing), and the
//! help/version text is wrapped with one blank line top and bottom.

use std::process::Command;

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Every hidden alias must resolve to its canonical command: invoking
/// `<alias> --help` exits 0 and prints that command's help (clap rejects an
/// unknown subcommand with a non-zero exit instead).
#[test]
fn aliases_resolve_to_their_command() {
	let cases = [
		("convert", "resolved compose file"),
		("gen", "declarative artifacts"),
		("resume", "Resume paused"),
		("remove", "Remove stopped"),
		("log", "output from containers"),
		("image", "images used by services"),
	];
	for (alias, needle) in cases {
		let output = Command::new(bin())
			.args([alias, "--help"])
			.output()
			.expect("run alias --help");
		assert!(
			output.status.success(),
			"alias `{alias}` did not resolve (exit {:?})",
			output.status.code()
		);
		let stdout = String::from_utf8_lossy(&output.stdout);
		assert!(
			stdout.contains(needle),
			"alias `{alias}` help missing {needle:?}; got:\n{stdout}"
		);
	}
}

/// Aliases are hidden: the top-level `--help` lists canonical commands only,
/// never the alias spellings.
#[test]
fn aliases_stay_out_of_top_level_help() {
	let output = Command::new(bin())
		.arg("--help")
		.output()
		.expect("run --help");
	let stdout = String::from_utf8_lossy(&output.stdout);
	for alias in ["convert", "resume", "remove"] {
		assert!(
			!stdout.lines().any(|l| l.trim_start().starts_with(alias)),
			"alias `{alias}` leaked into top-level help"
		);
	}
}

/// `--help` and `--version` are framed with exactly one blank line at the top
/// and bottom.
#[test]
fn help_and_version_are_blank_line_framed() {
	for flag in ["--help", "--version"] {
		let output = Command::new(bin()).arg(flag).output().expect("run flag");
		assert!(output.status.success());
		let stdout = String::from_utf8_lossy(&output.stdout);
		assert!(stdout.starts_with('\n'), "{flag}: no leading blank line");
		assert!(stdout.ends_with("\n\n"), "{flag}: no trailing blank line");
		assert!(
			!stdout.starts_with("\n\n"),
			"{flag}: more than one leading blank line"
		);
	}
}
