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
mod tests {
	use crate::cli::{Cli, Commands, PsFormat};
	use clap::Parser;

	#[test]
	fn ps_output_flags_conflict_and_ndjson_is_exclusive_to_ps() {
		for flags in [
			vec!["--all-services", "--quiet"],
			vec!["--all-services", "--services"],
			vec!["--all-services", "--format", "table"],
			vec!["--all-services", "--format", "json"],
			vec!["--all-services", "--format", "ndjson"],
			vec!["--services", "--format", "ndjson"],
			vec!["--quiet", "--format", "ndjson"],
		] {
			let mut args = vec!["podup", "ps"];
			args.extend(flags);
			let err = Cli::try_parse_from(args).err().expect("conflict");
			assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
		}
		let cli = Cli::try_parse_from(["podup", "ps", "--format", "ndjson"]).unwrap();
		assert!(matches!(cli.command, Commands::Ps(args) if args.format == PsFormat::Ndjson));
		let cli = Cli::try_parse_from(["podup", "ps", "--all-services", "web"]).unwrap();
		assert!(
			matches!(cli.command, Commands::Ps(args) if args.all_services && args.services == ["web"])
		);
		for cmd in ["ls", "images", "top", "stats"] {
			assert!(Cli::try_parse_from(["podup", cmd, "--format", "ndjson"]).is_err());
		}
	}
}
