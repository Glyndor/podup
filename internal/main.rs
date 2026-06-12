//! `podup` — docker-compose to Podman translator CLI.

use std::path::Path;
use std::process;

use clap::Parser;
use tracing_subscriber::EnvFilter;

mod cli;
mod resolve;

use cli::*;
use resolve::*;

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
				// Defense in depth: the unit stem is already sanitized in the
				// library, but never write a unit whose name is anything but a
				// plain file inside `dir` (rejects separators, `.` and `..`).
				if Path::new(&unit.filename).file_name()
					!= Some(std::ffi::OsStr::new(&unit.filename))
				{
					return Err(std::io::Error::new(
						std::io::ErrorKind::InvalidInput,
						format!("refusing unsafe quadlet unit file name: {}", unit.filename),
					)
					.into());
				}
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

	let compose_files = resolve_compose_files(&cli.file);
	let file = podup::parse_files_with_env_files(&compose_files, &cli.env_file)?;

	if matches!(cli.command, Commands::Config) {
		let yaml = serde_yaml::to_string(&file).map_err(podup::ComposeError::Parse)?;
		println!("{yaml}");
		return Ok(());
	}

	let base_dir = resolve_base_dir(cli.project_directory.as_deref(), &compose_files[0]);
	let project = resolve_project_name(cli.project, file.name.as_deref(), &base_dir);

	// `generate` produces declarative artifacts from the compose file alone; it
	// neither contacts Podman nor mutates project state.
	if let Commands::Generate {
		kind: GenerateCommands::Quadlet { output },
	} = &cli.command
	{
		return write_quadlet(&file, &project, output.as_deref());
	}

	let client = podup::podman::connect(cli.socket.as_deref())?;
	let engine = podup::Engine::with_base_dir(client, project, base_dir);

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
			no_deps,
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
					no_deps,
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
