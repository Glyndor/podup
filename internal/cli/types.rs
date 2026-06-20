//! Shared `ValueEnum` types for the CLI surface.

use clap::ValueEnum;

/// Output rendering for list commands (`ps`, `images`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
	/// Aligned columns for human reading.
	#[default]
	Table,
	/// Machine-readable JSON array.
	Json,
}

/// Which images `down --rmi` removes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum RmiScope {
	/// All images used by the project's services.
	All,
	/// Only images built locally from a service `build:` section.
	Local,
}

/// Output rendering for `config`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum ConfigFormat {
	/// YAML (the compose-file format).
	#[default]
	Yaml,
	/// JSON.
	Json,
}

/// Subcommands of `generate`.
#[derive(clap::Subcommand)]
pub(crate) enum GenerateCommands {
	/// Translate the compose file into Podman Quadlet unit files.
	///
	/// Emits one `.container` per service plus `.network` and `.volume` units.
	/// Without --output the units are printed to stdout; warnings about fields
	/// with no Quadlet mapping go to stderr.
	Quadlet {
		/// Directory to write the unit files into (e.g.
		/// ~/.config/containers/systemd). Prints to stdout when omitted.
		#[arg(short, long)]
		output: Option<std::path::PathBuf>,
	},
}
