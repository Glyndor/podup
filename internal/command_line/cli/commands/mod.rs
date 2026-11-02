//! The `podup` subcommand enum, split from `mod.rs` to stay within the
//! source line limit as the CLI surface grows.

use std::path::PathBuf;

use clap::Subcommand;
#[cfg(feature = "completions")]
use clap_complete::Shell;

use super::parse::{parse_progress, parse_pull_policy, parse_scale_pair, parse_timeout};
use super::types::{AuditFormat, AutostartCommands, GenerateCommands, OutputFormat, RmiScope};

mod create;
mod inspect;
mod interact;
pub(crate) use create::{RunArgs, UpArgs};
pub(crate) use inspect::{ConfigArgs, EventsArgs, LogsArgs, PsArgs, PsFormat};
pub(crate) use interact::{AttachArgs, CommitArgs, ExecArgs};

#[derive(Subcommand)]
#[non_exhaustive]
pub(crate) enum Commands {
	/// Create and start all services.
	Up(UpArgs),
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
		#[arg(short = 't', long, allow_hyphen_values = true, value_parser = parse_timeout)]
		timeout: Option<i32>,
	},
	/// Start existing stopped containers.
	Start {
		/// Wait until services are running/healthy before returning.
		#[arg(long)]
		wait: bool,
		/// Maximum seconds to wait with --wait before giving up.
		#[arg(long, requires = "wait")]
		wait_timeout: Option<u64>,
		/// Start only these services.
		services: Vec<String>,
	},
	/// Stop running containers without removing them.
	Stop {
		/// Seconds to wait for containers to stop before killing them.
		#[arg(short = 't', long, allow_hyphen_values = true, value_parser = parse_timeout)]
		timeout: Option<i32>,
		/// Stop only these services.
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
		#[arg(long, conflicts_with = "force_recreate")]
		no_recreate: bool,
		/// Pull policy before creating: always, missing, never, newer, build.
		#[arg(long, value_parser = parse_pull_policy)]
		pull: Option<String>,
		/// Do not create linked services (depends_on) of the named services.
		#[arg(long)]
		no_deps: bool,
		/// Create only these services.
		services: Vec<String>,
	},
	/// Display a live stream of container resource usage.
	Stats {
		/// Disable streaming; print a single snapshot and exit.
		#[arg(long)]
		no_stream: bool,
		/// Include non-running containers as zeroed rows.
		#[arg(short, long)]
		all: bool,
		/// Do not truncate long container names in the table.
		#[arg(long)]
		no_trunc: bool,
		/// Output format.
		#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
		format: OutputFormat,
		/// Show only these services.
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
		/// Filter projects by predicate: name=<NAME> or status=<running|exited>
		/// (repeatable).
		#[arg(long)]
		filter: Vec<String>,
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
		/// Suppress the push progress output.
		#[arg(short, long)]
		quiet: bool,
		/// Push only these services.
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
		/// Set the build progress output style (auto, plain, tty); accepted for
		/// compatibility with other Compose tools. Validated but inert: podup renders build
		/// output one way.
		#[arg(long, value_parser = parse_progress)]
		progress: Option<String>,
		/// Push each built image to its registry after a successful build.
		#[arg(long)]
		push: bool,
		/// Suppress the build output.
		#[arg(short, long)]
		quiet: bool,
		/// Build only these services.
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
		services: Vec<String>,
	},
	/// Pause running service containers.
	Pause {
		/// Pause only these services.
		services: Vec<String>,
	},
	/// Resume paused service containers.
	#[command(alias = "resume")]
	Unpause {
		/// Unpause only these services.
		services: Vec<String>,
	},
	/// Run a one-off command in a new service container.
	Run(RunArgs),
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
	Ps(PsArgs),
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
	Events(EventsArgs),
	/// Attach to a service container's output (stdout/stderr).
	Attach(AttachArgs),
	/// Block until service containers stop, printing each exit code.
	Wait {
		/// Output format. `json` emits one NDJSON object per container as it
		/// exits, rather than after every container has.
		#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
		format: OutputFormat,
		/// Wait on these services (default: all).
		services: Vec<String>,
	},
	/// Commit a service container to a new image.
	Commit(CommitArgs),
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
		/// Show each volume's size and how much removing it would reclaim.
		/// Slow: the only endpoint carrying a volume's size accounts for the
		/// whole host, which measured 1.2s against 10ms for the plain list.
		#[arg(short = 's', long)]
		size: bool,
		/// Output format.
		#[arg(long, value_enum, default_value_t = OutputFormat::Table)]
		format: OutputFormat,
		/// Show only volumes mounted by these services.
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
		/// Show images for these services only.
		services: Vec<String>,
	},
	/// View output from containers.
	#[command(alias = "log")]
	Logs(LogsArgs),
	/// Execute a command in a running service container.
	Exec(ExecArgs),
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
		#[arg(long, value_parser = parse_pull_policy)]
		policy: Option<String>,
		/// Only pull images for these services.
		services: Vec<String>,
	},
	/// Restart services.
	Restart {
		/// Seconds to wait for containers to stop before killing them.
		#[arg(short = 't', long, allow_hyphen_values = true, value_parser = parse_timeout)]
		timeout: Option<i32>,
		/// Do not restart dependent services (depends_on with a restart condition).
		#[arg(long)]
		no_deps: bool,
		/// Restart only these services (default: all).
		services: Vec<String>,
	},
	/// Print the resolved compose file (substitution/extends/include applied).
	#[command(alias = "convert")]
	Config(ConfigArgs),
	/// Audit the compose file for hardening gaps (read-only rootfs, dropped
	/// capabilities, memory/PID limits, host-binding modes, secret-shaped env
	/// vars, unpinned images, …) and print a row per service. No check changes
	/// what `up` does; this is a view, not a gate. `--strict` exits 1 when any
	/// finding is present, so it can fail CI. `--list-checks` enumerates the
	/// checks this build carries without reading a compose file, so an
	/// integrator can diff the set between releases.
	Audit {
		/// Exit 1 when any finding is present.
		#[arg(long)]
		strict: bool,
		/// Output format. Honoured by both the audit run and `--list-checks`,
		/// so `--format json` returns a `{"checks":[...]}` array of objects.
		#[arg(long, value_enum, default_value_t = AuditFormat::Table)]
		format: AuditFormat,
		/// List every check this build carries (one per line, id then
		/// description) and exit. Takes no compose file: the registry is the
		/// source, so `--strict` would have no effect and conflicts at parse
		/// time.
		#[arg(long, conflicts_with = "strict")]
		list_checks: bool,
		/// Enable the `port_published_on_wildcard` opt-in check, which
		/// fires for a port published with host IP `0.0.0.0` or `::`.
		/// Off by default so a CI that runs `audit --strict` today sees
		/// no new finding on upgrade; the check is the same one as the
		/// `port_published_on_wildcard` row in `audit --list-checks`.
		#[arg(long, conflicts_with = "list_checks")]
		wildcard_binds: bool,
	},
	/// Generate declarative artifacts from the compose file.
	#[command(
		alias = "gen",
		subcommand_required = true,
		arg_required_else_help = true
	)]
	Generate {
		#[command(subcommand)]
		kind: GenerateCommands,
	},
	/// Manage a boot-time autostart unit for this compose project (rootless,
	/// user-scope `systemctl --user`).
	#[command(
		alias = "boot",
		subcommand_required = true,
		arg_required_else_help = true
	)]
	Autostart {
		#[command(subcommand)]
		kind: AutostartCommands,
	},
	/// Print help for podup, or for a specific command.
	Help {
		/// Command to show help for. Extra tokens, `-h`/`--help`, and a leading
		/// `--` are tolerated; only the first command name is used.
		#[arg(trailing_var_arg = true, allow_hyphen_values = true)]
		commands: Vec<String>,
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
	/// Print version information.
	Version {
		/// Print only the version number.
		#[arg(long)]
		short: bool,
		/// Output format: pretty or json.
		#[arg(long, value_parser = ["pretty", "json"], default_value = "pretty")]
		format: String,
	},
	/// Print a shell completion script to stdout for the named shell.
	#[cfg(feature = "completions")]
	Completions {
		/// Shell to generate completions for (bash, zsh, fish, powershell, elvish).
		shell: Shell,
	},
}
