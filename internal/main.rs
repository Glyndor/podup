//! `podup` — docker-compose to Podman translator CLI.

use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
	name = "podup",
	version,
	about = "docker-compose translator for Podman"
)]
struct Cli {
	/// Path to the compose file. May also be set via `COMPOSE_FILE`. When
	/// unset, the compose-spec precedence list is probed in the current
	/// directory (compose.yaml, compose.yml, docker-compose.yaml,
	/// docker-compose.yml).
	#[arg(short, long, env = "COMPOSE_FILE")]
	file: Option<PathBuf>,

	/// Project name (used as a prefix for container names).
	/// May also be set via `COMPOSE_PROJECT_NAME`.
	#[arg(short, long, env = "COMPOSE_PROJECT_NAME", default_value = "podup")]
	project: String,

	/// Podman socket path (overrides auto-detection and PODMAN_SOCKET env).
	#[arg(long, env = "PODMAN_SOCKET")]
	socket: Option<String>,

	/// Active profiles (comma-separated).  May also be set via `COMPOSE_PROFILES`.
	#[arg(long, value_delimiter = ',', global = true)]
	profile: Vec<String>,

	/// Base directory for resolving relative paths (env_file, build context,
	/// bind mounts, config/secret file sources). Defaults to the directory
	/// containing the compose file.
	#[arg(long, global = true)]
	project_directory: Option<PathBuf>,

	#[command(subcommand)]
	command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
}

#[derive(Subcommand)]
enum GenerateCommands {
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

/// Whether a command creates, destroys, or changes the state of containers and
/// so must hold the exclusive project lock.
fn is_mutating(command: &Commands) -> bool {
	matches!(
		command,
		Commands::Up { .. }
			| Commands::Down { .. }
			| Commands::Start { .. }
			| Commands::Stop { .. }
			| Commands::Build { .. }
			| Commands::Rm { .. }
			| Commands::Kill { .. }
			| Commands::Pause { .. }
			| Commands::Unpause { .. }
			| Commands::Run { .. }
			| Commands::Restart { .. }
	)
}

/// Generate Quadlet units from the compose file and either write them to a
/// directory or print them to stdout. Warnings about unmapped fields go to
/// stderr so stdout stays clean for piping.
fn write_quadlet(
	file: &podup::compose::types::ComposeFile,
	project: &str,
	output: Option<&Path>,
) -> podup::Result<()> {
	let result = podup::quadlet::generate(file, project);
	for warning in &result.warnings {
		eprintln!("warning: {warning}");
	}
	match output {
		Some(dir) => {
			std::fs::create_dir_all(dir)?;
			for unit in &result.units {
				let path = dir.join(&unit.filename);
				std::fs::write(&path, &unit.contents)?;
				println!("wrote {}", path.display());
			}
		}
		None => {
			for unit in &result.units {
				println!("# {}", unit.filename);
				print!("{}", unit.contents);
				println!();
			}
		}
	}
	Ok(())
}

/// Compose-spec file-name precedence, highest first.
const COMPOSE_FILE_CANDIDATES: [&str; 4] = [
	"compose.yaml",
	"compose.yml",
	"docker-compose.yaml",
	"docker-compose.yml",
];

/// Resolve which compose file to load. An explicit `--file`/`COMPOSE_FILE`
/// wins; otherwise probe the compose-spec precedence list in the current
/// directory, falling back to `docker-compose.yml` so a missing-file error
/// names a sensible path.
fn resolve_compose_file(explicit: Option<PathBuf>) -> PathBuf {
	if let Some(path) = explicit {
		return path;
	}
	for candidate in COMPOSE_FILE_CANDIDATES {
		if Path::new(candidate).is_file() {
			return PathBuf::from(candidate);
		}
	}
	PathBuf::from("docker-compose.yml")
}

/// Resolve the base directory for relative-path resolution. An explicit
/// `--project-directory` wins; otherwise it is the directory containing the
/// compose file (compose-spec default), or the current directory when the
/// compose file has no parent component.
fn resolve_base_dir(project_directory: Option<&Path>, file: &Path) -> PathBuf {
	project_directory
		.map(Path::to_path_buf)
		.unwrap_or_else(|| file.parent().map(Path::to_path_buf).unwrap_or_default())
}

#[tokio::main]
async fn main() {
	match run().await {
		Ok(()) => {}
		Err(podup::ComposeError::RunExited(code)) => process::exit(code as i32),
		Err(e @ podup::ComposeError::Update(_)) => {
			eprintln!("error: {e}");
			process::exit(podup::update::exit_code(&e));
		}
		Err(e) => {
			eprintln!("error: {e}");
			process::exit(1);
		}
	}
}

async fn run() -> podup::Result<()> {
	tracing_subscriber::fmt()
		.with_env_filter(EnvFilter::from_default_env())
		.init();

	let cli = Cli::parse();

	// `update` operates on the binary itself, not a compose project, so it runs
	// before any compose file is parsed or Podman is contacted. The network and
	// filesystem work is blocking; keep it off the async path entirely.
	if let Commands::Update { check, force } = cli.command {
		let opts = podup::update::UpdateOptions {
			check_only: check,
			force,
		};
		return tokio::task::spawn_blocking(move || podup::update::run(opts))
			.await
			.map_err(|e| podup::ComposeError::Update(format!("update task failed: {e}")))?;
	}

	let compose_path = resolve_compose_file(cli.file.clone());
	let file = podup::parse_file(&compose_path)?;

	if matches!(cli.command, Commands::Config) {
		let yaml = serde_yaml::to_string(&file).map_err(podup::ComposeError::Parse)?;
		println!("{yaml}");
		return Ok(());
	}

	// `generate` produces declarative artifacts from the compose file alone; it
	// neither contacts Podman nor mutates project state.
	if let Commands::Generate {
		kind: GenerateCommands::Quadlet { output },
	} = &cli.command
	{
		return write_quadlet(&file, &cli.project, output.as_deref());
	}

	let client = podup::podman::connect(cli.socket.as_deref())?;
	let base_dir = resolve_base_dir(cli.project_directory.as_deref(), &compose_path);
	let engine = podup::Engine::with_base_dir(client, cli.project, base_dir);

	// Serialize mutating lifecycle commands against concurrent `podup` runs on
	// the same project. Read-only / follow commands (ps, logs, top, port,
	// images, exec, pull, cp, config, watch) take no lock so they don't block
	// or get blocked. The guard is held until `run` returns.
	let _lock = if is_mutating(&cli.command) {
		Some(engine.lock_project()?)
	} else {
		None
	};

	match cli.command {
		Commands::Up {
			detach,
			build,
			watch,
			remove_orphans,
			no_recreate,
			force_recreate,
			services,
		} => {
			if remove_orphans {
				engine.remove_orphans(&file).await?;
			}
			if build {
				engine.build_all(&file, &services).await?;
			}
			engine
				.up_with_options(
					&file,
					detach,
					&cli.profile,
					&services,
					no_recreate,
					force_recreate,
				)
				.await?;
			if watch {
				engine.watch(&file).await?;
			} else if !detach {
				engine.attach_logs(&file).await?;
				let _ = engine.stop(&file, &[]).await;
			}
		}
		Commands::Down { volumes } => engine.down_with_options(&file, volumes).await?,
		Commands::Start { services } => engine.start(&file, &services).await?,
		Commands::Stop { services } => engine.stop(&file, &services).await?,
		Commands::Build { services } => engine.build_all(&file, &services).await?,
		Commands::Rm { force, services } => engine.rm(&file, &services, force).await?,
		Commands::Kill { signal, services } => engine.kill(&file, &services, &signal).await?,
		Commands::Pause { services } => engine.pause(&file, &services).await?,
		Commands::Unpause { services } => engine.unpause(&file, &services).await?,
		Commands::Run {
			service,
			rm,
			detach,
			env_overrides,
			name,
			service_ports,
			cmd,
		} => {
			engine
				.run(
					&file,
					&service,
					podup::RunOptions {
						cmd,
						rm,
						detach,
						env_overrides,
						name_override: name,
						service_ports,
					},
				)
				.await?
		}
		Commands::Cp { src, dst } => engine.cp(&file, &src, &dst).await?,
		Commands::Ps => engine.ps(&file).await?,
		Commands::Top { services } => engine.top(&file, &services).await?,
		Commands::Port {
			service,
			private_port,
			proto,
		} => engine.port(&file, &service, private_port, &proto).await?,
		Commands::Images => engine.images(&file).await?,
		Commands::Logs { service, follow } => {
			engine.logs(&file, service.as_deref(), follow).await?
		}
		Commands::Exec { service, cmd } => engine.exec(&file, &service, cmd).await?,
		Commands::Pull => engine.pull(&file).await?,
		Commands::Restart { service } => engine.restart(&file, service.as_deref()).await?,
		Commands::Config => unreachable!("handled above"),
		Commands::Generate { .. } => unreachable!("handled above"),
		Commands::Watch => engine.watch(&file).await?,
		Commands::Update { .. } => unreachable!("handled before compose parsing"),
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{resolve_base_dir, resolve_compose_file};
	use std::path::{Path, PathBuf};

	#[test]
	fn explicit_compose_file_wins() {
		let p = resolve_compose_file(Some(PathBuf::from("custom.yml")));
		assert_eq!(p, PathBuf::from("custom.yml"));
	}

	#[test]
	fn missing_compose_file_falls_back_to_default_name() {
		// In a directory with no candidate files, the default name is returned
		// so the resulting error names a sensible path.
		let dir = std::env::temp_dir().join(format!("podup-cf-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let prev = std::env::current_dir().unwrap();
		std::env::set_current_dir(&dir).unwrap();
		let p = resolve_compose_file(None);
		std::env::set_current_dir(prev).unwrap();
		let _ = std::fs::remove_dir_all(&dir);
		assert_eq!(p, PathBuf::from("docker-compose.yml"));
	}

	#[test]
	fn project_directory_override_wins() {
		let base = resolve_base_dir(
			Some(Path::new("/srv/app")),
			Path::new("/etc/compose/docker-compose.yml"),
		);
		assert_eq!(base, PathBuf::from("/srv/app"));
	}

	#[test]
	fn defaults_to_compose_file_parent() {
		let base = resolve_base_dir(None, Path::new("/etc/compose/docker-compose.yml"));
		assert_eq!(base, PathBuf::from("/etc/compose"));
	}

	#[test]
	fn bare_filename_resolves_to_current_dir() {
		let base = resolve_base_dir(None, Path::new("docker-compose.yml"));
		assert_eq!(base, PathBuf::from(""));
	}
}
