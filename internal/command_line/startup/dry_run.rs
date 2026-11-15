//! Which commands may run under `--dry-run`.
//!
//! The list is an allowlist: a command that only reads, or that already has a
//! way to show what it would do, runs; every other command, including one added
//! later, refuses. `--dry-run` that a command ignored would change things while
//! looking safe, so a refusal is the only wrong-looking answer that is right.

use clap::CommandFactory;

use crate::cli::{AutostartCommands, Cli, Commands, GenerateCommands};

/// `None` when the command may run under `--dry-run`, otherwise the message
/// that explains why it does not.
pub(crate) fn refusal(cli: &Cli) -> Option<String> {
	refusal_for(cli, std::env::args_os())
}

/// [`refusal`] against an explicit argument list, which a test can supply.
pub(crate) fn refusal_for(
	cli: &Cli,
	args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Option<String> {
	if !cli.dry_run || allowed(&cli.command) {
		return None;
	}
	Some(format!(
		"--dry-run is not supported for `{}`: it would change something, and the flag \
		 does not stop it yet. Nothing was run. Use a read-only command (`ps`, `config`, \
		 `ls`, `images`, `volumes`, `audit`), or `autostart install --dry-run`.",
		command_name(args)
	))
}

fn allowed(command: &Commands) -> bool {
	match command {
		Commands::Ls { .. }
		| Commands::Stats { .. }
		| Commands::Ps(_)
		| Commands::Top { .. }
		| Commands::Events(_)
		| Commands::Wait { .. }
		| Commands::Port { .. }
		| Commands::Volumes { .. }
		| Commands::Images { .. }
		| Commands::Logs(_)
		| Commands::Config(_)
		| Commands::Audit { .. }
		| Commands::Help { .. }
		| Commands::Version { .. } => true,
		#[cfg(feature = "completions")]
		Commands::Completions { .. } => true,
		// Prints the tar stream on stdout; `-o` writes a file on the host.
		Commands::Export { output, .. } => output.is_none(),
		// Prints the units on stdout; `-o` writes them into a directory.
		Commands::Generate {
			kind: GenerateCommands::Quadlet { output },
		} => output.is_none(),
		// `install` shows the unit and the actions; `status` only reads.
		Commands::Autostart { kind } => {
			matches!(
				kind,
				AutostartCommands::Install { .. } | AutostartCommands::Status
			)
		}
		_ => false,
	}
}

/// The command as the operator typed it (`down`, `autostart uninstall`), read
/// back from clap so the message never names a different command.
fn command_name(args: impl IntoIterator<Item = std::ffi::OsString>) -> String {
	let Ok(matches) = Cli::command().try_get_matches_from(args) else {
		return "this command".to_string();
	};
	match matches.subcommand() {
		Some((name, sub)) => match sub.subcommand_name() {
			Some(inner) if name == "autostart" || name == "generate" => format!("{name} {inner}"),
			_ => name.to_string(),
		},
		None => "this command".to_string(),
	}
}

#[cfg(test)]
mod tests;
