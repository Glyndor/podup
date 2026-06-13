//! Command-line interface definitions for `podup`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser)]
#[command(
	name = "podup",
	version,
	about = "docker-compose translator for Podman"
)]
pub(crate) struct Cli {
	/// Path to the compose file. May also be set via `COMPOSE_FILE`. When
	/// unset, the compose-spec precedence list is probed in the current
	/// directory (compose.yaml, compose.yml, docker-compose.yaml,
	/// docker-compose.yml).
	#[arg(short, long)]
	pub(crate) file: Vec<PathBuf>,

	/// Project name (used as a prefix for container names). May also be set via
	/// `COMPOSE_PROJECT_NAME`. When unset, the compose-spec precedence applies:
	/// the top-level `name:` field, then the sanitized basename of the project
	/// directory.
	#[arg(short, long, env = "COMPOSE_PROJECT_NAME")]
	pub(crate) project: Option<String>,

	/// Podman socket path (overrides auto-detection and PODMAN_SOCKET env).
	#[arg(long, env = "PODMAN_SOCKET")]
	pub(crate) socket: Option<String>,

	/// Active profiles (comma-separated).  May also be set via `COMPOSE_PROFILES`.
	#[arg(long, value_delimiter = ',', global = true)]
	pub(crate) profile: Vec<String>,

	/// Base directory for resolving relative paths (env_file, build context,
	/// bind mounts, config/secret file sources). Defaults to the directory
	/// containing the compose file.
	#[arg(long, global = true)]
	pub(crate) project_directory: Option<PathBuf>,

	/// Additional env file(s) loaded into the variable map used for
	/// interpolation. May be given multiple times; later files win. The
	/// process environment and a project `.env` still take precedence.
	#[arg(long = "env-file", global = true)]
	pub(crate) env_file: Vec<String>,

	#[command(subcommand)]
	pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
	/// Create and start all services.
	Up {
		/// Run containers in the background.
		#[arg(short, long)]
		detach: bool,
		/// Build images before starting containers.
		#[arg(long)]
		build: bool,
		/// Watch for file changes and sync/rebuild/restart per develop.watch rules.
		#[arg(short, long)]
		watch: bool,
		/// Remove containers for services not defined in the compose file.
		#[arg(long)]
		remove_orphans: bool,
		/// Do not recreate containers that are already running.
		#[arg(long)]
		no_recreate: bool,
		/// Recreate containers even if their configuration is unchanged.
		#[arg(long)]
		force_recreate: bool,
		/// Do not start linked services (depends_on) of the named services.
		#[arg(long)]
		no_deps: bool,
		/// Bring up only these services (and their transitive depends_on).
		/// If omitted, brings up every service in the compose file.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Stop and remove containers.
	Down {
		/// Also remove named volumes declared in the compose file.
		#[arg(short = 'v', long)]
		volumes: bool,
	},
	/// Start existing stopped containers.
	Start {
		/// Start only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Stop running containers without removing them.
	Stop {
		/// Stop only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Build or rebuild service images.
	Build {
		/// Build only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Remove stopped service containers.
	Rm {
		/// Remove even running containers (stop first).
		#[arg(short, long)]
		force: bool,
		/// Remove only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Send a signal to service containers.
	Kill {
		/// Signal to send (default: SIGKILL).
		#[arg(short, long, default_value = "SIGKILL")]
		signal: String,
		/// Signal only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Pause running service containers.
	Pause {
		/// Pause only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Resume paused service containers.
	Unpause {
		/// Unpause only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Run a one-off command in a new service container.
	Run {
		/// Service to run the command against.
		service: String,
		/// Remove the container after it exits (default: true).
		#[arg(long, default_value_t = true)]
		rm: bool,
		/// Run container in the background.
		#[arg(short, long)]
		detach: bool,
		/// Set environment variables (KEY=VAL).
		#[arg(short, long = "env")]
		env_overrides: Vec<String>,
		/// Override the container name.
		#[arg(long)]
		name: Option<String>,
		/// Publish the service's declared ports (off by default).
		#[arg(long)]
		service_ports: bool,
		/// Command (and arguments) to run.
		#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
		cmd: Vec<String>,
	},
	/// Copy files between a service container and the local filesystem.
	///
	/// Use SERVICE:PATH for the container side (e.g. `web:/app/data ./local`).
	Cp {
		/// Source path. Use SERVICE:PATH for a container path.
		src: String,
		/// Destination path. Use SERVICE:PATH for a container path.
		dst: String,
	},
	/// List containers.
	Ps,
	/// Display the running processes of service containers.
	Top {
		/// Show only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Print the public port for a port binding of a service container.
	Port {
		/// Service name.
		service: String,
		/// Private port number.
		private_port: u16,
		/// Protocol (tcp or udp).
		#[arg(long, default_value = "tcp")]
		proto: String,
	},
	/// List images used by services.
	Images,
	/// View output from containers.
	Logs {
		/// Only show logs for this service.
		service: Option<String>,
		/// Follow log output.
		#[arg(short, long)]
		follow: bool,
	},
	/// Execute a command in a running service container.
	Exec {
		/// Service name.
		service: String,
		/// Command (and arguments) to execute.
		#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
		cmd: Vec<String>,
	},
	/// Pull images for all services.
	Pull,
	/// Restart services.
	Restart {
		/// Only restart this service.
		service: Option<String>,
	},
	/// Print the resolved compose file (after substitution / extends / include).
	Config,
	/// Generate declarative artifacts from the compose file.
	Generate {
		#[command(subcommand)]
		kind: GenerateCommands,
	},
	/// Watch for file changes and sync/rebuild/restart as configured by develop.watch.
	Watch,
	/// Update podup to the latest signed release.
	///
	/// Downloads the release binary for this platform and replaces the running
	/// executable, but only after verifying the release's Ed25519 signature
	/// against the public key embedded in this build and matching its SHA-256
	/// checksum. Verification fails closed: a missing key, bad signature, or
	/// checksum mismatch aborts without touching the installed binary.
	Update {
		/// Report whether a newer release exists without installing it.
		#[arg(long)]
		check: bool,
		/// Reinstall even if the latest release is not newer than this build.
		#[arg(long)]
		force: bool,
	},
	/// Print a shell completion script to stdout.
	///
	/// Generates a completion script for the named shell from the CLI
	/// definition. Source it from your shell's startup (or install the file the
	/// Debian package ships) to get tab completion for podup commands and flags.
	Completions {
		/// Shell to generate completions for (bash, zsh, fish, powershell, elvish).
		shell: Shell,
	},
}

#[derive(Subcommand)]
pub(crate) enum GenerateCommands {
	/// Translate the compose file into Podman Quadlet unit files.
	///
	/// Emits one `.container` per service plus `.network` and `.volume` units.
	/// Without --output the units are printed to stdout; warnings about fields
	/// with no Quadlet mapping go to stderr.
	Quadlet {
		/// Directory to write the unit files into (e.g.
		/// ~/.config/containers/systemd). Prints to stdout when omitted.
		#[arg(short, long)]
		output: Option<PathBuf>,
	},
}
