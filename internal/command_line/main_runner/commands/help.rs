//! `help [COMMAND]` rendering for the CLI.
//!
//! The static clap definition supplies the help text; this module finds the
//! deepest subcommand the user named and renders it. Help-flag tokens
//! (`-h`/`--help`) and a leading `--` are tolerated, and an unknown
//! subcommand falls back to the top-level help.

use crate::cli::Cli;

/// Render help for `help [COMMAND]`, framed and colour-aware to match clap's own
/// `--help`. Help-flag tokens (`-h`/`--help`) and a leading `--` are tolerated;
/// the first remaining token selects the subcommand (an unknown one falls back
/// to the top-level help).
pub(crate) fn print_command_help(commands: &[String]) -> podup::Result<()> {
	use clap::CommandFactory;
	let mut cmd = Cli::command();
	let target = commands
		.iter()
		.find(|t| !matches!(t.as_str(), "-h" | "--help" | "--"));
	let rendered = match target.and_then(|name| cmd.find_subcommand_mut(name)) {
		Some(sub) => sub.render_long_help(),
		None => cmd.render_long_help(),
	};
	if podup::ui::stdout_colored() {
		print!("\n{}\n", rendered.ansi());
	} else {
		print!("\n{rendered}\n");
	}
	Ok(())
}
