//! The `podup` subcommand enum, split from `mod.rs` to stay within the
//! source line limit as the CLI surface grows.

use std::path::PathBuf;

use clap::Subcommand;
#[cfg(feature = "completions")]
use clap_complete::Shell;

use super::parse::parse_scale_pair;
use super::types::{ConfigFormat, EventsFormat, GenerateCommands, OutputFormat, RmiScope};

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
		/// Watch and sync/rebuild/restart per develop.watch rules.
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
		/// Seconds to wait for a container to stop when recreating.
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
		/// Wait until services are running/healthy before returning.
		#[arg(long)]
		wait: bool,
		/// Create containers but do not start them.
		#[arg(long)]
		no_start: bool,
		/// Prefix attached log lines with a timestamp (ignored with -d).
		#[arg(long)]
		timestamps: bool,
		/// Recreate anonymous volumes instead of keeping the previous ones.
		#[arg(short = 'V', long)]
		renew_anon_volumes: bool,
		/// Bring up only these services (and their depends_on); default: all.
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
		/// Also remove service images: `all` or `local` (build-section services).
		#[arg(long, value_enum)]
		rmi: Option<RmiScope>,
		/// Seconds to wait for containers to stop before killing them.
		#[arg(short = 't', long)]
		timeout: Option<i32>,
	},
	/// Start existing stopped containers.
	Start {
		/// Wait until services are running/healthy before returning.
		#[arg(long)]
		wait: bool,
		/// Maximum seconds to wait with --wait before giving up.
		#[arg(long)]
		wait_timeout: Option<u64>,
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
		/// Only display project names. Mutually exclusive with `--format`.
		#[arg(short, long, conflicts_with = "format")]
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
		/// Verify the registry TLS cert (false for insecure/HTTP; default on).
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
		/// Also remove anonymous volumes attached to the containers.
		#[arg(short = 'v', long)]
		volumes: bool,
		/// Stop the containers (gracefully) before removing them.
		#[arg(short = 's', long)]
		stop: bool,
		/// Remove only these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Send a signal to service containers.
	Kill {
		/// Signal to send (default: SIGKILL).
		#[arg(short, long, default_value = "SIGKILL")]
		signal: String,
		/// Also remove containers for services not in the compose file.
		#[arg(long)]
		remove_orphans: bool,
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
		/// Remove the container after it exits (the default).
		#[arg(long, overrides_with = "no_rm")]
		rm: bool,
		/// Keep the one-off container after it exits instead of removing it.
		#[arg(long, overrides_with = "rm")]
		no_rm: bool,
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
		#[arg(short = 'P', long)]
		service_ports: bool,
		/// Run the command as this user (`name or UID[:GID]`).
		#[arg(short, long)]
		user: Option<String>,
		/// Working directory inside the container.
		#[arg(short, long)]
		workdir: Option<String>,
		/// Override the image entrypoint.
		#[arg(long)]
		entrypoint: Option<String>,
		/// Bind-mount an extra volume (HOST:CONTAINER[:OPTS] or NAME:CONTAINER); repeatable.
		#[arg(short = 'v', long = "volume")]
		volume: Vec<String>,
		/// Publish an extra port (HOST:CONTAINER[/PROTO]); repeatable.
		#[arg(short = 'p', long = "publish")]
		publish: Vec<String>,
		/// Keep the container's STDIN open (sets `stdin_open`). `run` streams the
		/// container's output but does not attach a live interactive terminal.
		#[arg(short, long)]
		interactive: bool,
		/// No effect; accepted only for docker-compose compatibility. podup never
		/// allocates a pseudo-TTY.
		#[arg(short = 'T', long = "no-TTY")]
		no_tty: bool,
		/// Do not start linked services (depends_on) before running.
		#[arg(long)]
		no_deps: bool,
		/// Command (and arguments) to run.
		#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
		cmd: Vec<String>,
	},
	/// Copy files between a service container and the host (SERVICE:PATH for the
	/// container side, e.g. `web:/app/data ./local`).
	Cp {
		/// Source path. Use SERVICE:PATH for a container path.
		src: String,
		/// Destination path. Use SERVICE:PATH for a container path.
		dst: String,
		/// Target this replica index (1-based) of a scaled service.
		#[arg(long)]
		index: Option<u32>,
		/// Follow symlinks in the host source before copying into the container.
		#[arg(short = 'L', long)]
		follow_link: bool,
		/// Archive mode (accepted for compatibility; no effect under rootless Podman).
		#[arg(short = 'a', long)]
		archive: bool,
	},
	/// List containers.
	Ps {
		/// Show all containers, including stopped ones.
		#[arg(short, long)]
		all: bool,
		/// Only display container IDs. Mutually exclusive with `--format`.
		#[arg(short, long, conflicts_with = "format")]
		quiet: bool,
		/// Output format.
		#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
		format: OutputFormat,
	},
	/// Display the running processes of service containers.
	Top {
		/// Output format.
		#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
		format: OutputFormat,
		/// Show only these services. Unlike the other service-list commands, `top`
		/// takes a plain positional (not `trailing_var_arg`) so `--format` parses
		/// in any position (`top web --format json` as well as `top --format json
		/// web`); service names are never hyphen-prefixed, so nothing is lost.
		services: Vec<String>,
	},
	/// Stream Podman events for this project's containers.
	Events {
		/// Output format: a `TYPE ACTION NAME` summary (table) or one JSON
		/// object per line (NDJSON, not a JSON array).
		#[arg(long, value_enum, default_value_t = EventsFormat::Table)]
		format: EventsFormat,
		/// Deprecated alias for `--format json`; kept for backward compatibility.
		#[arg(long, hide = true)]
		json: bool,
	},
	/// Attach to a service container's output (stdout/stderr).
	Attach {
		/// Service whose container to attach to.
		service: String,
	},
	/// Block until service containers stop, printing each exit code.
	Wait {
		/// Wait on these services (default: all).
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Commit a service container to a new image.
	Commit {
		/// Service whose container to commit.
		service: String,
		/// Target image reference (repo[:tag]).
		image: String,
		/// Replica index (1-based) of a scaled service.
		#[arg(long)]
		index: Option<u32>,
		/// Pause the container during commit for a consistent snapshot
		/// (default on; `--pause=false` to snapshot a live container).
		#[arg(
			long,
			default_value_t = true,
			action = clap::ArgAction::Set,
			num_args = 0..=1,
			default_missing_value = "true",
		)]
		pause: bool,
	},
	/// Export a service container's filesystem as a tar archive.
	Export {
		/// Service whose container to export.
		service: String,
		/// Write to this file instead of stdout.
		#[arg(short, long)]
		output: Option<PathBuf>,
		/// Replica index (1-based) of a scaled service.
		#[arg(long)]
		index: Option<u32>,
	},
	/// Print the public port for a port binding of a service container.
	Port {
		/// Service name.
		service: String,
		/// Private port, e.g. `80` or `80/tcp` (a `/proto` suffix overrides --protocol).
		private_port: String,
		/// Protocol (tcp or udp).
		#[arg(long, visible_alias = "protocol", default_value = "tcp")]
		proto: String,
		/// Index of the container when the service has multiple replicas (1-based).
		#[arg(long)]
		index: Option<u32>,
	},
	/// List the project's named volumes.
	#[command(alias = "volume")]
	Volumes {
		/// Only display volume names. Mutually exclusive with `--format`.
		#[arg(short, long, conflicts_with = "format")]
		quiet: bool,
		/// Output format.
		#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
		format: OutputFormat,
		/// Show only volumes mounted by these services.
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// List images used by services.
	#[command(alias = "image")]
	Images {
		/// Only display image IDs. Mutually exclusive with `--format`.
		#[arg(short, long, conflicts_with = "format")]
		quiet: bool,
		/// Output format.
		#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
		format: OutputFormat,
	},
	/// View output from containers.
	#[command(alias = "log")]
	Logs {
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
		/// Show logs for these services (default: all).
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Execute a command in a running service container.
	Exec {
		/// Set environment variables (KEY=VAL); may be repeated.
		#[arg(short, long = "env")]
		env: Vec<String>,
		/// Run the command as this user (`name or UID[:GID]`).
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
		/// No effect; accepted only for docker-compose compatibility. podup never
		/// allocates a pseudo-TTY.
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
		/// Suppress image-pull progress output.
		#[arg(short, long)]
		quiet: bool,
		/// Continue pulling the remaining services after a failure.
		#[arg(long)]
		ignore_pull_failures: bool,
		/// Also pull images for the named services' depends_on services.
		#[arg(long)]
		include_deps: bool,
		/// Pull policy: always, missing, never, newer, build (overrides per-service pull_policy).
		#[arg(long)]
		policy: Option<String>,
		/// Only pull images for these services.
		#[arg(trailing_var_arg = true)]
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
		/// Restart only these services (default: all).
		#[arg(trailing_var_arg = true)]
		services: Vec<String>,
	},
	/// Print the resolved compose file (substitution/extends/include applied).
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
		/// Leave ${VAR} placeholders literal instead of interpolating them.
		#[arg(long)]
		no_interpolate: bool,
		/// Rewrite each service image to its registry digest (repo@sha256:...).
		#[arg(long)]
		resolve_image_digests: bool,
	},
	/// Generate declarative artifacts from the compose file.
	#[command(alias = "gen")]
	Generate {
		#[command(subcommand)]
		kind: GenerateCommands,
	},
	/// Watch for file changes and sync/rebuild/restart as configured by develop.watch.
	Watch,
	/// Update podup to the latest signed release (Ed25519 signature + SHA-256
	/// verified against the embedded key; fails closed, leaving the binary
	/// untouched on any mismatch).
	#[cfg(feature = "update")]
	Update {
		/// Report whether a newer release exists without installing it.
		#[arg(long)]
		check: bool,
		/// Reinstall even if the latest release is not newer than this build.
		#[arg(long)]
		force: bool,
	},
	/// Print a shell completion script to stdout for the named shell.
	#[cfg(feature = "completions")]
	Completions {
		/// Shell to generate completions for (bash, zsh, fish, powershell, elvish).
		shell: Shell,
	},
}
