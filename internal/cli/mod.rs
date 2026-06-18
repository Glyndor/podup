//! Command-line interface definitions for `podup`.

use std::path::PathBuf;

use clap::Parser;

mod commands;
mod parse;
mod types;
pub(crate) use commands::Commands;
pub(crate) use types::{ConfigFormat, GenerateCommands, OutputFormat, RmiScope};

#[derive(Parser)]
#[command(name = "podup", version)]
pub(crate) struct Cli {
	/// Path to the compose file (or `COMPOSE_FILE`). Unset: probe the
	/// compose-spec precedence list (compose.yaml/.yml, docker-compose.yaml/.yml).
	#[arg(short, long)]
	pub(crate) file: Vec<PathBuf>,

	/// Project name, the container-name prefix (or `COMPOSE_PROJECT_NAME`).
	/// Unset: the top-level `name:`, then the sanitized project-directory basename.
	#[arg(short, long, env = "COMPOSE_PROJECT_NAME")]
	pub(crate) project: Option<String>,

	/// Podman socket path (overrides auto-detection and PODMAN_SOCKET env).
	#[arg(long, env = "PODMAN_SOCKET")]
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

	#[command(subcommand)]
	pub(crate) command: Commands,
}
