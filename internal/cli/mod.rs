//! Command-line interface definitions for `podup`.

use std::path::PathBuf;

use clap::Parser;

mod commands;
mod parse;
mod types;
pub(crate) use commands::Commands;
pub(crate) use types::{AnsiMode, ConfigFormat, GenerateCommands, OutputFormat, RmiScope};

/// Top-level clap parser for the `podup` CLI; fields carry the per-flag docs.
#[derive(Parser)]
#[command(name = "podup", version)]
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

	/// Extra env file(s) for interpolation (repeatable, later win; process env and `.env` still win).
	#[arg(long = "env-file", global = true)]
	pub(crate) env_file: Vec<String>,

	/// When to colourise output: auto (TTY only), always, or never. `NO_COLOR`
	/// also forces plain output.
	#[arg(long, value_enum, default_value_t = AnsiMode::Auto, global = true)]
	pub(crate) ansi: AnsiMode,

	#[command(subcommand)]
	pub(crate) command: Commands,
}
