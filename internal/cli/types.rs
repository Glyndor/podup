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
	/// Only images without a custom tag (services that build locally).
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
