//! `podup` — docker-compose to Podman translator CLI.

use std::path::PathBuf;
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
	/// Path to the compose file.
	#[arg(short, long, default_value = "docker-compose.yml")]
	file: PathBuf,

	/// Project name (used as a prefix for container names).
	#[arg(short, long, default_value = "podup")]
	project: String,

	/// Podman socket path (overrides auto-detection and PODMAN_SOCKET env).
	#[arg(long, env = "PODMAN_SOCKET")]
	socket: Option<String>,

	/// Active profiles (comma-separated).  May also be set via `COMPOSE_PROFILES`.
	#[arg(long, value_delimiter = ',', global = true)]
	profile: Vec<String>,

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
	/// List containers.
	Ps,
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
	/// Watch for file changes and sync/rebuild/restart as configured by develop.watch.
	Watch,
}

#[tokio::main]
async fn main() {
	if let Err(e) = run().await {
		eprintln!("error: {e}");
		process::exit(1);
	}
}

async fn run() -> podup::Result<()> {
	tracing_subscriber::fmt()
		.with_env_filter(EnvFilter::from_default_env())
		.init();

	let cli = Cli::parse();

	let file = podup::parse_file(&cli.file)?;

	// The `config` command does not need a Podman connection.
	if matches!(cli.command, Commands::Config) {
		let yaml = serde_yaml::to_string(&file).map_err(podup::ComposeError::Parse)?;
		println!("{yaml}");
		return Ok(());
	}

	let docker = podup::podman::connect(cli.socket.as_deref())?;
	let base_dir = cli
		.file
		.parent()
		.map(|p| p.to_path_buf())
		.unwrap_or_default();
	let engine = podup::Engine::with_base_dir(docker, cli.project, base_dir);

	match cli.command {
		Commands::Up {
			detach,
			build,
			watch,
			remove_orphans,
			no_recreate,
			services,
		} => {
			if remove_orphans {
				engine.remove_orphans(&file).await?;
			}
			if build {
				engine.build_all(&file, &services).await?;
			}
			engine
				.up_with_options(&file, detach, &cli.profile, &services, no_recreate)
				.await?;
			if watch {
				engine.watch(&file).await?;
			} else if !detach {
				engine.attach_logs(&file).await?;
			}
		}
		Commands::Down { volumes } => engine.down_with_options(&file, volumes).await?,
		Commands::Start { services } => engine.start(&file, &services).await?,
		Commands::Stop { services } => engine.stop(&file, &services).await?,
		Commands::Build { services } => engine.build_all(&file, &services).await?,
		Commands::Rm { force, services } => engine.rm(&file, &services, force).await?,
		Commands::Kill { signal, services } => engine.kill(&file, &services, &signal).await?,
		Commands::Ps => engine.ps(&file).await?,
		Commands::Logs { service, follow } => {
			engine.logs(&file, service.as_deref(), follow).await?
		}
		Commands::Exec { service, cmd } => engine.exec(&file, &service, cmd).await?,
		Commands::Pull => engine.pull(&file).await?,
		Commands::Restart { service } => engine.restart(&file, service.as_deref()).await?,
		Commands::Config => unreachable!("handled above"),
		Commands::Watch => engine.watch(&file).await?,
	}

	Ok(())
}
