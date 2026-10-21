//! Arguments of `commit`: the image a container is committed to.

use clap::Args;

#[derive(Args)]
pub(crate) struct CommitArgs {
	/// Service whose container to commit.
	pub(crate) service: String,
	/// Target image reference (repo[:tag]).
	pub(crate) image: String,
	/// Commit message for the new image.
	#[arg(short, long)]
	pub(crate) message: Option<String>,
	/// Author of the new image (e.g. "Name <email>").
	#[arg(short, long)]
	pub(crate) author: Option<String>,
	/// Apply a Containerfile instruction to the committed image, e.g.
	/// `CMD=/bin/sh` or `ENV=KEY=value` (repeatable).
	#[arg(short = 'c', long = "change")]
	pub(crate) change: Vec<String>,
	/// Replica index (1-based) of a scaled service.
	#[arg(long)]
	pub(crate) index: Option<u32>,
	/// Pause the container during commit for a consistent snapshot
	/// (default on; `--pause=false` to snapshot a live container).
	#[arg(
		short,
		long,
		default_value_t = true,
		action = clap::ArgAction::Set,
		num_args = 0..=1,
		default_missing_value = "true",
	)]
	pub(crate) pause: bool,
}
