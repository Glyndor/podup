//! Arguments of `config`: which projection of the resolved compose file is
//! printed, and the switches that shape it.

use clap::Args;

use crate::cli::ConfigFormat;

#[derive(Args)]
pub(crate) struct ConfigArgs {
	/// Output format.
	#[arg(long, value_enum, default_value_t = ConfigFormat::Yaml)]
	pub(crate) format: ConfigFormat,
	/// Print only the service names, one per line.
	#[arg(long)]
	pub(crate) services: bool,
	/// Print only the named volumes, one per line.
	#[arg(long)]
	pub(crate) volumes: bool,
	/// Print only the service image references, one per line.
	#[arg(long)]
	pub(crate) images: bool,
	/// Print only the profile names, one per line.
	#[arg(long)]
	pub(crate) profiles: bool,
	/// Print the config hash of all services, or of the given comma-separated
	/// services ("*" for all).
	#[arg(long)]
	pub(crate) hash: Option<String>,
	/// Only validate the configuration; print nothing.
	#[arg(short, long)]
	pub(crate) quiet: bool,
	/// Leave ${VAR} placeholders literal instead of interpolating them.
	#[arg(long)]
	pub(crate) no_interpolate: bool,
	/// Accepted for compatibility; podup output is already
	/// normalized.
	#[arg(long)]
	pub(crate) no_normalize: bool,
	/// Rewrite each service image to its registry digest (repo@sha256:...).
	#[arg(long)]
	pub(crate) resolve_image_digests: bool,
}
