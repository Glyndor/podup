//! Command-line interface definitions for `podup`.

use std::path::PathBuf;

use clap::Parser;

mod commands;
mod parse;
mod types;
pub(crate) use commands::Commands;
pub(crate) use types::{
	AnsiMode, AutostartCommands, AutostartMode, ConfigFormat, EventsFormat, GenerateCommands,
	OutputFormat, RmiScope,
};

/// Help-screen colours (clap honours its own TTY/`NO_COLOR`/`CLICOLOR` detection
/// for these, since help is rendered before `--ansi` is parsed): green-bold
/// section headers and usage, cyan-bold flag/command literals, plain placeholders.
const HELP_STYLES: clap::builder::Styles = clap::builder::Styles::plain()
	.header(clap::builder::styling::AnsiColor::Green.on_default().bold())
	.usage(clap::builder::styling::AnsiColor::Green.on_default().bold())
	.literal(clap::builder::styling::AnsiColor::Cyan.on_default().bold())
	.placeholder(clap::builder::styling::AnsiColor::Cyan.on_default());

/// Top-level clap parser for the `podup` CLI; fields carry the per-flag docs.
//
// An explicit `about` is set on `#[command]` so clap does not promote this
// internal doc comment to the program's `--help` description.
#[derive(Parser)]
#[command(
	name = "podup",
	version,
	about = "Run docker-compose projects on Podman.",
	styles = HELP_STYLES,
	// No subcommand prints help and exits non-zero (like docker compose), and the
	// built-in `help` is replaced by an explicit `Help` variant that tolerates
	// extra tokens, `-h`/`--help`, and a leading `--`.
	arg_required_else_help = true,
	disable_help_subcommand = true
)]
pub(crate) struct Cli {
	/// Path to the compose file (or `COMPOSE_FILE`). Unset: probe the
	/// compose-spec precedence list (compose.yaml/.yml, docker-compose.yaml/.yml).
	// Not `global`: its `-f` short would collide with subcommand `-f` flags
	// (e.g. `rm --force`), which clap forbids. Must precede the subcommand.
	#[arg(short, long)]
	pub(crate) file: Vec<PathBuf>,

	/// Project name, the container-name prefix (or `COMPOSE_PROJECT_NAME`).
	/// Unset: the top-level `name:`, then the sanitized project-directory basename.
	// Not `global`: its `-p` short would collide with subcommand `-p` flags
	// (e.g. `run --publish`). Must precede the subcommand.
	#[arg(short, long, env = "COMPOSE_PROJECT_NAME")]
	pub(crate) project: Option<String>,

	/// Podman socket path (overrides auto-detection and PODMAN_SOCKET env).
	/// `global` so it can appear before or after the subcommand (it has no
	/// short flag, so there is no collision).
	#[arg(long, env = "PODMAN_SOCKET", global = true)]
	pub(crate) socket: Option<String>,

	/// Active profiles (comma-separated).  May also be set via `COMPOSE_PROFILES`.
	#[arg(long, value_delimiter = ',', env = "COMPOSE_PROFILES", global = true)]
	pub(crate) profile: Vec<String>,

	/// Base directory for relative paths (env_file, build context, bind mounts,
	/// config/secret sources). Defaults to the compose file's directory.
	#[arg(long, global = true)]
	pub(crate) project_directory: Option<PathBuf>,

	/// Extra env file(s) for interpolation (repeatable; later `--env-file` wins, and these replace the default `.env`; process env still wins).
	/// With `run`, they also seed the one-off container's environment (below `environment:`/`-e`).
	#[arg(long = "env-file", global = true)]
	pub(crate) env_file: Vec<String>,

	/// When to colourise output: auto (TTY only), always, or never. With `auto`,
	/// `NO_COLOR` also forces plain output; `--ansi always` overrides `NO_COLOR`.
	#[arg(long, value_enum, default_value_t = AnsiMode::Auto, global = true)]
	pub(crate) ansi: AnsiMode,

	#[command(subcommand)]
	pub(crate) command: Commands,
}
