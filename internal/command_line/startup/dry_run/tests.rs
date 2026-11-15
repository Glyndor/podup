//! `--dry-run` runs a command that only reads and refuses every other one.

use std::ffi::OsString;

use clap::Parser;

use super::*;

fn parse(line: &str) -> (Cli, Vec<OsString>) {
	let args: Vec<OsString> = std::iter::once("podup")
		.chain(line.split_whitespace())
		.map(OsString::from)
		.collect();
	(Cli::try_parse_from(&args).expect("the line parses"), args)
}

fn verdict(line: &str) -> Option<String> {
	let (cli, args) = parse(line);
	refusal_for(&cli, args)
}

#[test]
fn a_command_without_the_flag_is_never_refused() {
	for line in ["up", "down -v", "rm", "pull", "exec web ls", "cp a web:/b"] {
		assert!(
			verdict(line).is_none(),
			"{line} was refused without --dry-run"
		);
	}
}

#[test]
fn the_commands_that_only_read_run() {
	for line in [
		"--dry-run ps",
		"--dry-run config",
		"--dry-run ls",
		"--dry-run images",
		"--dry-run volumes",
		"--dry-run audit",
		"--dry-run version",
		"--dry-run logs web",
		"--dry-run top web",
		"--dry-run port web 80",
		"--dry-run export web",
		"--dry-run generate quadlet",
		"--dry-run autostart status",
		"--dry-run autostart install",
		"autostart install --dry-run",
		"ps --dry-run",
	] {
		assert!(verdict(line).is_none(), "{line} was refused");
	}
}

#[test]
fn every_command_that_changes_something_is_refused() {
	for line in [
		"--dry-run up",
		"--dry-run down",
		"--dry-run down -v",
		"--dry-run create",
		"--dry-run run web ls",
		"--dry-run start",
		"--dry-run stop",
		"--dry-run restart",
		"--dry-run rm",
		"--dry-run kill",
		"--dry-run pause",
		"--dry-run unpause",
		"--dry-run scale web=2",
		"--dry-run build",
		"--dry-run pull",
		"--dry-run push",
		"--dry-run cp web:/a b",
		"--dry-run exec web ls",
		"--dry-run attach web",
		"--dry-run watch",
		"--dry-run autostart uninstall",
	] {
		let message = verdict(line).unwrap_or_else(|| panic!("{line} was not refused"));
		assert!(message.contains("--dry-run"), "{line}: {message}");
	}
}

#[test]
fn a_command_that_would_write_a_file_is_refused_with_an_output_path() {
	assert!(verdict("--dry-run export web -o out.tar").is_some());
	assert!(verdict("--dry-run generate quadlet -o units").is_some());
}

#[test]
fn the_message_names_the_command_the_operator_typed() {
	let message = verdict("--dry-run down -v").expect("refused");
	assert!(message.contains("`down`"), "{message}");
	let message = verdict("--dry-run autostart uninstall").expect("refused");
	assert!(message.contains("`autostart uninstall`"), "{message}");
}
