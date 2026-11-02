//! Arguments of `ps`: which containers are listed and how the table is shaped.

use clap::{Args, ValueEnum};

/// Output formats exclusive to `ps`; other commands retain `OutputFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum PsFormat {
	Table,
	Json,
	Ndjson,
}

impl std::fmt::Display for PsFormat {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Table => "table",
			Self::Json => "json",
			Self::Ndjson => "ndjson",
		})
	}
}

#[derive(Args)]
pub(crate) struct PsArgs {
	/// Show all containers, including stopped ones.
	#[arg(short, long)]
	pub(crate) all: bool,
	/// Only display container IDs. Mutually exclusive with `--format` and
	/// `--services`.
	#[arg(short, long, conflicts_with_all = ["format", "services_only", "all_services"])]
	pub(crate) quiet: bool,
	/// Print the service names, one per line, instead of the container
	/// table. Mutually exclusive with `--quiet` and `--format`.
	#[arg(long = "services", conflicts_with_all = ["format", "quiet", "all_services"])]
	pub(crate) services_only: bool,
	/// Print configured service names in file order, including those without containers.
	#[arg(long, conflicts_with_all = ["quiet", "format", "services_only"])]
	pub(crate) all_services: bool,
	/// Show each container's on-disk size. Off by default: the server has
	/// to walk each container's writable layer to answer, which measured
	/// 21ms to 109ms over 59 containers.
	#[arg(short = 's', long)]
	pub(crate) size: bool,
	/// Filter containers by predicate: status=<running|exited> or
	/// name=<NAME> (repeatable).
	#[arg(long)]
	pub(crate) filter: Vec<String>,
	/// Filter by container status (running, exited, ...); repeatable.
	#[arg(long)]
	pub(crate) status: Vec<String>,
	/// Output format: `table` (aligned columns), `json` (a JSON array), or `ndjson` (one object per line).
	/// Mutually exclusive with `--quiet` and `--services`.
	#[arg(long, value_enum, default_value_t = PsFormat::Table)]
	pub(crate) format: PsFormat,
	/// Show only these services.
	pub(crate) services: Vec<String>,
}

#[cfg(test)]
#[path = "ps_tests.rs"]
mod tests;
