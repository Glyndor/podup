//! Command-line interface definitions for `podup`.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
#[cfg(feature = "completions")]
use clap_complete::Shell;

/// Output rendering for list commands (`ps`, `images`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
	/// Aligned columns for human reading.
	#[default]
	Table,
	/// Machine-readable JSON array.
	Json,
}

/// Output rendering for `config`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum ConfigFormat {
	/// YAML (the compose-file format).
	#[default]
	Yaml,
	/// JSON.
	Json,
}

#[derive(Parser)]
#[command(name = "podup", version)]
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
	#[arg(long, value_delimiter = ',', env = "COMPOSE_PROFILES", global = true)]
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
		/// Seconds to wait for containers to stop when recreating, before killing them.
		#[arg(short = 't', long)]
		timeout: Option<i32>,
		/// Override the replica count for a service: SERVICE=N (repeatable).
		#[arg(long, value_parser = parse_scale_pair)]
		scale: Vec<(String, u32)>,
		/// Pull policy before starting: always, missing, never.
		#[arg(long)]
		pull: Option<String>,
		/// Do not build images, even for services with a build section.
		#[arg(long)]
		no_build: bool,
		/// Suppress image-pull progress output.
		#[arg(long)]
		quiet_pull: bool,
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
		/// Remove containers for services not defined in the compose file.
		#[arg(long)]
		remove_orphans: bool,
		/// Seconds to wait for containers to stop before killing them.
		#[arg(short = 't', long)]
		timeout: Option<i32>,
	},
	/// Start existing stopped containers.
	Start {
		/// Start only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Stop running containers without removing them.
	Stop {
		/// Seconds to wait for containers to stop before killing them.
		#[arg(short = 't', long)]
		timeout: Option<i32>,
		/// Stop only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Set the number of running containers for services.
	Scale {
		/// Target replica counts: SERVICE=N (one or more).
		#[arg(value_parser = parse_scale_pair, required = true)]
		pairs: Vec<(String, u32)>,
	},
	/// Create containers for services without starting them.
	Create {
		/// Build images before creating containers.
		#[arg(long)]
		build: bool,
		/// Recreate containers even if their configuration is unchanged.
		#[arg(long)]
		force_recreate: bool,
		/// Do not recreate containers that already exist.
		#[arg(long)]
		no_recreate: bool,
		/// Create only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Display a live stream of container resource usage.
	Stats {
		/// Disable streaming; print a single snapshot and exit.
		#[arg(long)]
		no_stream: bool,
		/// Show only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// List podup compose projects on the host.
	Ls {
		/// Show all projects, including stopped ones.
		#[arg(short, long)]
		all: bool,
		/// Only display project names.
		#[arg(short, long)]
		quiet: bool,
		/// Output format.
		#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
		format: OutputFormat,
	},
	/// Push service images to their registry.
	Push {
		/// Continue pushing the remaining services after a failure.
		#[arg(long)]
		ignore_push_failures: bool,
		/// Verify the registry's TLS certificate (set false for an insecure
		/// or local HTTP registry). Omitted leaves Podman's default (on).
		#[arg(long)]
		tls_verify: Option<bool>,
		/// Push only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Build or rebuild service images.
	Build {
		/// Do not use cache when building the image.
		#[arg(long)]
		no_cache: bool,
		/// Always attempt to pull a newer version of the base image.
		#[arg(long)]
		pull: bool,
		/// Set build-time variables (KEY=VAL); may be repeated.
		#[arg(long = "build-arg")]
		build_arg: Vec<String>,
		/// Suppress the build output.
		#[arg(short, long)]
		quiet: bool,
		/// Build only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Remove stopped service containers.
	#[command(alias = "remove")]
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
	#[command(alias = "resume")]
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
	Ps {
		/// Show all containers, including stopped ones.
		#[arg(short, long)]
		all: bool,
		/// Only display container IDs.
		#[arg(short, long)]
		quiet: bool,
		/// Output format.
		#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
		format: OutputFormat,
	},
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
	#[command(alias = "image")]
	Images {
		/// Only display image IDs.
		#[arg(short, long)]
		quiet: bool,
		/// Output format.
		#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
		format: OutputFormat,
	},
	/// View output from containers.
	#[command(alias = "log")]
	Logs {
		/// Only show logs for this service.
		service: Option<String>,
		/// Follow log output.
		#[arg(short, long)]
		follow: bool,
		/// Number of lines to show from the end of the logs (default: all).
		#[arg(short = 'n', long)]
		tail: Option<String>,
		/// Show logs since a timestamp or relative time (e.g. 2024-01-01T00:00:00, 10m).
		#[arg(long)]
		since: Option<String>,
		/// Show logs before a timestamp or relative time.
		#[arg(long)]
		until: Option<String>,
		/// Prefix each line with an RFC3339 timestamp.
		#[arg(short = 't', long)]
		timestamps: bool,
	},
	/// Execute a command in a running service container.
	Exec {
		/// Set environment variables (KEY=VAL); may be repeated.
		#[arg(short, long = "env")]
		env: Vec<String>,
		/// Run the command as this user (name or UID[:GID]).
		#[arg(short, long)]
		user: Option<String>,
		/// Working directory inside the container.
		#[arg(short, long)]
		workdir: Option<String>,
		/// Give extended privileges to the command.
		#[arg(long)]
		privileged: bool,
		/// Detach: run the command in the background.
		#[arg(short, long)]
		detach: bool,
		/// Disable pseudo-TTY allocation (podup never allocates one; accepted for compatibility).
		#[arg(short = 'T', long = "no-TTY")]
		no_tty: bool,
		/// Index of the container when the service has multiple replicas (1-based).
		#[arg(long)]
		index: Option<u32>,
		/// Service name.
		service: String,
		/// Command (and arguments) to execute.
		#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
		cmd: Vec<String>,
	},
	/// Pull images for the named services, or all services if none are given.
	Pull {
		/// Only pull images for these services.
		services: Vec<String>,
	},
	/// Restart services.
	Restart {
		/// Seconds to wait for containers to stop before killing them.
		#[arg(short = 't', long)]
		timeout: Option<i32>,
		/// Do not restart dependent services (depends_on with a restart condition).
		#[arg(long)]
		no_deps: bool,
		/// Only restart this service.
		service: Option<String>,
	},
	/// Print the resolved compose file (after substitution / extends / include).
	#[command(alias = "convert")]
	Config {
		/// Output format.
		#[arg(long, value_enum, default_value_t = ConfigFormat::Yaml)]
		format: ConfigFormat,
		/// Print only the service names, one per line.
		#[arg(long)]
		services: bool,
		/// Only validate the configuration; print nothing.
		#[arg(short, long)]
		quiet: bool,
	},
	/// Generate declarative artifacts from the compose file.
	#[command(alias = "gen")]
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
	#[cfg(feature = "update")]
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
	#[cfg(feature = "completions")]
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

/// Parse a `SERVICE=N` scale argument into a `(service, replicas)` pair.
///
/// Rejects a missing `=`, an empty service name, a non-numeric count, and `N=0`
/// (use `down`/`stop` to remove a service, not `scale=0`).
fn parse_scale_pair(value: &str) -> Result<(String, u32), String> {
	let (service, count) = value
		.split_once('=')
		.ok_or_else(|| format!("expected SERVICE=N, got `{value}`"))?;
	if service.is_empty() {
		return Err(format!("missing service name in `{value}`"));
	}
	let replicas: u32 = count
		.parse()
		.map_err(|_| format!("replica count in `{value}` must be a non-negative integer"))?;
	if replicas == 0 {
		return Err(format!(
			"replica count in `{value}` must be at least 1; use `down`/`stop` to remove a service"
		));
	}
	Ok((service.to_string(), replicas))
}

#[cfg(test)]
mod tests {
	use super::parse_scale_pair;

	#[test]
	fn parse_scale_pair_accepts_valid() {
		assert_eq!(parse_scale_pair("web=3"), Ok(("web".to_string(), 3)));
	}

	#[test]
	fn parse_scale_pair_rejects_bad_input() {
		for bad in ["web", "=3", "web=", "web=x", "web=0", "web=-1"] {
			assert!(parse_scale_pair(bad).is_err(), "`{bad}` should be rejected");
		}
	}
}
