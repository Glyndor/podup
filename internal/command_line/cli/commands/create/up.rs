//! Arguments of `up`: the flags that decide how the whole project is created
//! and started.

use clap::Args;

use crate::cli::parse::{parse_pull_policy, parse_scale_pair, parse_timeout};

#[derive(Args)]
pub(crate) struct UpArgs {
	/// Run containers in the background.
	#[arg(short, long)]
	pub(crate) detach: bool,
	/// Build images before starting containers.
	#[arg(long, conflicts_with = "no_build")]
	pub(crate) build: bool,
	/// Watch and sync/rebuild/restart per develop.watch rules.
	#[arg(short, long)]
	pub(crate) watch: bool,
	/// Remove containers for services not defined in the compose file.
	#[arg(long)]
	pub(crate) remove_orphans: bool,
	/// Do not recreate containers that are already running.
	#[arg(long, conflicts_with = "force_recreate")]
	pub(crate) no_recreate: bool,
	/// Recreate containers even if their configuration is unchanged.
	#[arg(long)]
	pub(crate) force_recreate: bool,
	/// Do not start linked services (depends_on) of the named services.
	#[arg(long)]
	pub(crate) no_deps: bool,
	/// Seconds to wait for a container to stop when recreating.
	#[arg(short = 't', long, allow_hyphen_values = true, value_parser = parse_timeout)]
	pub(crate) timeout: Option<i32>,
	/// Override the replica count for a service: SERVICE=N (repeatable).
	#[arg(long, value_parser = parse_scale_pair)]
	pub(crate) scale: Vec<(String, u32)>,
	/// Pull policy before starting: always, missing, never, newer, build.
	#[arg(long, value_parser = parse_pull_policy)]
	pub(crate) pull: Option<String>,
	/// Do not build images, even for services with a build section.
	#[arg(long)]
	pub(crate) no_build: bool,
	/// Suppress image-pull progress output.
	#[arg(long)]
	pub(crate) quiet_pull: bool,
	/// Wait until services are running/healthy before returning.
	#[arg(long)]
	pub(crate) wait: bool,
	/// Maximum seconds to wait with --wait before giving up.
	#[arg(long, requires = "wait")]
	pub(crate) wait_timeout: Option<u64>,
	/// Create containers but do not start them.
	#[arg(long)]
	pub(crate) no_start: bool,
	/// Prefix attached log lines with a timestamp (ignored with -d).
	#[arg(long)]
	pub(crate) timestamps: bool,
	/// Recreate anonymous volumes instead of keeping the previous ones.
	#[arg(short = 'V', long)]
	pub(crate) renew_anon_volumes: bool,
	/// Stop every container as soon as any container exits. Cannot be combined
	/// with `-d/--detach`, `--wait` or `--watch`: they all change what
	/// "attaching" means, and a foreground abort is the only one the flag
	/// applies to.
	#[arg(long, conflicts_with_all = ["detach", "wait", "watch"])]
	pub(crate) abort_on_container_exit: bool,
	/// Return the exit code of the named service's container as podup's own
	/// exit status. Implies `--abort-on-container-exit`, and is rejected
	/// with the same combination rule as that flag.
	#[arg(
		long,
		value_name = "SERVICE",
		conflicts_with_all = ["detach", "wait", "watch"]
	)]
	pub(crate) exit_code_from: Option<String>,
	/// Bring up only these services (and their depends_on); default: all.
	pub(crate) services: Vec<String>,
}
