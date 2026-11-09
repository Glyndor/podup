//! `help [COMMAND]` rendering for the CLI.
//!
//! The static clap definition supplies the help text; this module finds the
//! deepest subcommand the user named and renders it. Help-flag tokens
//! (`-h`/`--help`) and a leading `--` are tolerated, and an unknown
//! subcommand falls back to the top-level help.

use crate::cli::Cli;

/// The help of the deepest subcommand `commands` names, or the top-level help
/// when the first name is not a subcommand. Help-flag tokens (`-h`/`--help`) and
/// a leading `--` are skipped.
///
/// The command is built first, as the parser does before it runs: building
/// gives every subcommand its full name (`podup generate quadlet`, so the
/// `Usage:` line is one that can be typed) and applies the root's
/// `disable_help_subcommand` to the nested ones, which otherwise list a `help`
/// subcommand that `podup generate help` then rejects.
pub(crate) fn command_help(commands: &[String]) -> clap::builder::StyledStr {
	use clap::CommandFactory;
	let mut root = Cli::command();
	root.build();
	// The names that lead somewhere, found without borrowing mutably.
	let mut path = Vec::new();
	let mut probe = &root;
	for token in commands
		.iter()
		.filter(|t| !matches!(t.as_str(), "-h" | "--help" | "--"))
	{
		match probe.find_subcommand(token) {
			Some(sub) => {
				path.push(token.as_str());
				probe = sub;
			}
			None => break,
		}
	}
	let mut current = &mut root;
	for name in path {
		current = current
			.find_subcommand_mut(name)
			.expect("the name was found a moment ago");
	}
	current.render_long_help()
}

/// Render help for `help [COMMAND...]`, framed and colour-aware to match clap's
/// own `--help`.
pub(crate) fn print_command_help(commands: &[String]) -> podup::Result<()> {
	let rendered = command_help(commands);
	if podup::ui::stdout_colored() {
		podup::ui::out!("\n{}\n", rendered.ansi())?;
	} else {
		podup::ui::out!("\n{rendered}\n")?;
	}
	Ok(())
}

#[cfg(test)]
#[path = "help_tests.rs"]
mod tests;
