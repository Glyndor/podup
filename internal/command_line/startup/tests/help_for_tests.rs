use super::help_for;
use crate::cli::Cli;
use clap::CommandFactory;

fn help(args: &[&str]) -> String {
	help_for(Cli::command(), args.iter().map(|s| (*s).to_string())).to_string()
}

#[test]
fn no_subcommand_renders_the_root_help() {
	assert!(help(&[]).contains("Usage: podup [OPTIONS] <COMMAND>"));
}

#[test]
fn a_subcommand_group_renders_its_own_help_not_the_root() {
	let out = help(&["generate"]);
	assert!(
		out.contains("Usage: podup generate"),
		"expected generate's own usage, got: {out}"
	);
	// The point of the fix: the root's usage must NOT be what a user asking
	// about `generate` is shown.
	assert!(
		!out.contains("Usage: podup [OPTIONS] <COMMAND>"),
		"the root help was rendered instead: {out}"
	);
	assert!(
		out.contains("quadlet"),
		"the group's subcommands are missing"
	);
}

#[test]
fn an_alias_resolves_to_the_command_it_names() {
	assert!(help(&["gen"]).contains("Usage: podup generate"));
}

#[test]
fn flags_before_the_subcommand_do_not_stop_the_walk() {
	// `--ansi never generate` must still reach generate. A walk that treated
	// the flag's value as a subcommand token would stop at `never`.
	assert!(help(&["--ansi", "never", "autostart"]).contains("Usage: podup autostart"));
}

#[test]
fn a_flag_value_that_looks_like_a_subcommand_is_not_walked_into() {
	// `-p` names a project. A project called "build" is ordinary, and reading
	// it as the `build` command would render the wrong help for bare
	// `podup -p build`.
	let out = help(&["-p", "build"]);
	assert!(
		out.contains("Usage: podup [OPTIONS] <COMMAND>"),
		"a flag's value was walked into as a subcommand: {out}"
	);
}

#[test]
fn the_equals_form_carries_its_own_value() {
	// `--project-name=build` consumes nothing after it, so `generate` is still
	// the next token to consider.
	assert!(help(&["--project-name=build", "generate"]).contains("Usage: podup generate"));
}

#[test]
fn an_unknown_token_stops_the_walk_at_the_last_real_command() {
	assert!(help(&["generate", "nosuchthing"]).contains("Usage: podup generate"));
}
