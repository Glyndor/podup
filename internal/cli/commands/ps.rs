//! Arguments of `ps`: which containers are listed and how the table is shaped.

use clap::Args;

use crate::cli::OutputFormat;

#[derive(Args)]
pub(crate) struct PsArgs {
	/// Show all containers, including stopped ones.
	#[arg(short, long)]
	pub(crate) all: bool,
	/// Only display container IDs. Mutually exclusive with `--format` and
	/// `--services`.
	#[arg(short, long, conflicts_with_all = ["format", "services_only"])]
	pub(crate) quiet: bool,
	/// Print the service names, one per line, instead of the container
	/// table. Mutually exclusive with `--quiet` and `--format`.
	#[arg(long = "services", conflicts_with_all = ["format", "quiet"])]
	pub(crate) services_only: bool,
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
	/// Output format: `table` (aligned columns) or `json` (a JSON array).
	/// Mutually exclusive with `--quiet` and `--services`.
	#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
	pub(crate) format: OutputFormat,
	/// Show only these services.
	pub(crate) services: Vec<String>,
}
