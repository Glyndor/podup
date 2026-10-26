//! Arguments of `run`: the one-off container a command runs in.

use clap::Args;

#[derive(Args)]
pub(crate) struct RunArgs {
	/// Service to run the command against.
	pub(crate) service: String,
	/// Remove the container after it exits (the default).
	#[arg(long, overrides_with = "no_rm")]
	pub(crate) rm: bool,
	/// Keep the one-off container after it exits instead of removing it.
	#[arg(long, overrides_with = "rm")]
	pub(crate) no_rm: bool,
	/// Run container in the background.
	#[arg(short, long)]
	pub(crate) detach: bool,
	/// Set environment variables (KEY=VAL).
	#[arg(short, long = "env")]
	pub(crate) env_overrides: Vec<String>,
	/// Override the container name.
	#[arg(long)]
	pub(crate) name: Option<String>,
	/// Publish the service's declared ports (off by default).
	#[arg(short = 'P', long)]
	pub(crate) service_ports: bool,
	/// Run the command as this user (`name or UID[:GID]`).
	#[arg(short, long)]
	pub(crate) user: Option<String>,
	/// Working directory inside the container.
	#[arg(short, long)]
	pub(crate) workdir: Option<String>,
	/// Override the image entrypoint.
	#[arg(long)]
	pub(crate) entrypoint: Option<String>,
	/// Bind-mount an extra volume (HOST:CONTAINER[:OPTS] or NAME:CONTAINER); repeatable.
	#[arg(short = 'v', long = "volume")]
	pub(crate) volume: Vec<String>,
	/// Publish an extra port (HOST:CONTAINER[/PROTO]); repeatable.
	#[arg(short = 'p', long = "publish")]
	pub(crate) publish: Vec<String>,
	/// Keep the container's STDIN open (sets `stdin_open`). Whether a live
	/// terminal is attached is decided by stdin and stdout both being
	/// terminals, and by `-T`, not by this flag.
	#[arg(short, long)]
	pub(crate) interactive: bool,
	/// Disable pseudo-TTY allocation. `run` allocates one when stdin and
	/// stdout are both terminals; this opts out, so a script keeps plain
	/// streaming.
	// Both spellings are accepted on purpose: `docker compose run` spells the
	// long form `--no-TTY` while `docker compose exec` spells it `--no-tty`.
	// A script copied from either one has to work, so each command takes both.
	#[arg(short = 'T', long = "no-TTY", visible_alias = "no-tty")]
	pub(crate) no_tty: bool,
	/// Do not start linked services (depends_on) before running.
	#[arg(long)]
	pub(crate) no_deps: bool,
	/// Add a label to the one-off container (KEY=VAL); may be repeated.
	#[arg(short = 'l', long = "label")]
	pub(crate) label: Vec<String>,
	/// Command (and arguments) to run.
	#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
	pub(crate) cmd: Vec<String>,
}
