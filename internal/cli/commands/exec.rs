//! Arguments of `exec`: the command run inside an already running container.

use clap::Args;

#[derive(Args)]
pub(crate) struct ExecArgs {
	/// Set environment variables (KEY=VAL); may be repeated.
	#[arg(short, long = "env")]
	pub(crate) env: Vec<String>,
	/// Run the command as this user (`name or UID[:GID]`).
	#[arg(short, long)]
	pub(crate) user: Option<String>,
	/// Working directory inside the container.
	#[arg(short, long)]
	pub(crate) workdir: Option<String>,
	/// Give extended privileges to the command.
	#[arg(long)]
	pub(crate) privileged: bool,
	/// Detach: run the command in the background.
	#[arg(short, long)]
	pub(crate) detach: bool,
	/// Disable pseudo-TTY allocation. `exec` allocates one when stdin and
	/// stdout are both terminals; this opts out, so a script keeps plain
	/// streaming.
	// `docker compose exec` spells the long form `--no-tty`, so that is the
	// primary here; `--no-TTY` stays accepted so the spelling `run` uses also
	// works on `exec`. See the note on `run`'s field.
	#[arg(short = 'T', long = "no-tty", visible_alias = "no-TTY")]
	pub(crate) no_tty: bool,
	/// Index of the container when the service has multiple replicas (1-based).
	#[arg(long)]
	pub(crate) index: Option<u32>,
	/// Service name.
	pub(crate) service: String,
	/// Command (and arguments) to execute.
	#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
	pub(crate) cmd: Vec<String>,
}
