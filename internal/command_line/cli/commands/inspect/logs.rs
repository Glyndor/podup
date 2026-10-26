//! Arguments of `logs`: which slice of a service's log stream is replayed and
//! how each line is rendered.

use clap::Args;

#[derive(Args)]
pub(crate) struct LogsArgs {
	/// Follow log output.
	#[arg(short, long)]
	pub(crate) follow: bool,
	/// Number of lines to show from the end of the logs (default: 100; `all` for the full stream).
	#[arg(short = 'n', long)]
	pub(crate) tail: Option<String>,
	/// Show logs since a timestamp or relative time (e.g.
	/// 2024-01-01T00:00:00, -10m).
	#[arg(long, allow_hyphen_values = true)]
	pub(crate) since: Option<String>,
	/// Show logs before a timestamp or relative time (e.g. -5m).
	#[arg(long, allow_hyphen_values = true)]
	pub(crate) until: Option<String>,
	/// Prefix each line with an RFC3339 timestamp.
	#[arg(short = 't', long)]
	pub(crate) timestamps: bool,
	/// Produce monochrome output (no colour in the service-name prefix).
	#[arg(long)]
	pub(crate) no_color: bool,
	/// Do not print the service-name prefix on each line.
	#[arg(long)]
	pub(crate) no_log_prefix: bool,
	/// Show logs for these services (default: all).
	#[arg(trailing_var_arg = true)]
	pub(crate) services: Vec<String>,
}
