//! `podup` — docker-compose to Podman translator CLI.

// The binary carries no `unsafe`; deny it so any future addition is caught.
#![deny(unsafe_code)]

use std::process;

#[cfg(feature = "completions")]
use clap::CommandFactory;

mod cli;
mod generate;
mod resolve;
mod startup;

use cli::*;
use generate::write_quadlet;
use resolve::*;
use startup::{init_tracing, internal_error_notice, parse_cli};

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
			| Commands::Scale { .. }
			| Commands::Create { .. }
	)
}

fn main() {
	// Replace the default panic output (a raw Rust backtrace) with a `podup:`
	// internal-error notice that tells the user what to report and where, plus
	// the reminder to redact secrets first.
	std::panic::set_hook(Box::new(|info| {
		eprintln!("podup: internal error: {info}");
		eprintln!("{}", internal_error_notice());
	}));

	// Drive the runtime on a worker thread with a large stack. Clap's
	// command-building (debug builds especially) is stack-heavy and overflows
	// Windows' 1 MiB main-thread stack as the subcommand surface grows; an 8 MiB
	// matches Linux's default and leaves ample headroom.
	std::thread::Builder::new()
		.stack_size(8 * 1024 * 1024)
		.name("podup".into())
		.spawn(run_to_exit)
		.expect("spawn podup worker thread")
		.join()
		.expect("podup worker thread panicked");
}

/// Build the Tokio runtime and drive [`run`], mapping its result onto the
/// process exit status. Runs on the large-stack worker thread spawned by `main`.
fn run_to_exit() {
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.expect("build Tokio runtime");
	match runtime.block_on(run()) {
		Ok(()) => {}
		Err(podup::ComposeError::RunExited(code)) => process::exit(code as i32),
		#[cfg(feature = "update")]
		Err(e @ podup::ComposeError::Update(_)) => {
			eprintln!("podup: error: {e}");
			process::exit(podup::update::exit_code(&e));
		}
		Err(e) => {
			eprintln!("podup: error: {e}");
			process::exit(1);
		}
	}
}

async fn run() -> podup::Result<()> {
	init_tracing();
	let cli = parse_cli();

	// `completions` derives entirely from the static CLI definition; it neither
	// parses a compose file nor contacts Podman. Print to stdout for piping.
	#[cfg(feature = "completions")]
	if let Commands::Completions { shell } = cli.command {
		let mut cmd = Cli::command();
		let name = cmd.get_name().to_string();
		clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
		return Ok(());
	}

	// `update` operates on the binary itself, not a compose project, so it runs
	// before any compose file is parsed or Podman is contacted. The network and
	// filesystem work is blocking; keep it off the async path entirely.
	#[cfg(feature = "update")]
	if let Commands::Update { check, force } = cli.command {
		let opts = podup::update::UpdateOptions {
			check_only: check,
			force,
		};
		return tokio::task::spawn_blocking(move || podup::update::run(opts))
			.await
			.map_err(|e| podup::ComposeError::Update(format!("update task failed: {e}")))?;
	}

	// `ls` discovers projects across the host by container label; it needs a
	// Podman connection but no compose file, so handle it before parsing one.
	if let Commands::Ls { all, quiet, format } = &cli.command {
		let client = podup::podman::connect(cli.socket.as_deref())?;
		return podup::list_projects(
			&client,
			podup::LsOptions {
				all: *all,
				quiet: *quiet,
				json: *format == OutputFormat::Json,
			},
		)
		.await;
	}

	let compose_files = resolve_compose_files(&cli.file);
	let file = podup::parse_files_with_env_files(&compose_files, &cli.env_file)?;

	if matches!(cli.command, Commands::Config) {
		let mut redacted = file.clone();
		redacted.redact_inline_content();
		let yaml = serde_yaml::to_string(&redacted).map_err(podup::ComposeError::Parse)?;
		println!("{yaml}");
		return Ok(());
	}

	let base_dir = resolve_base_dir(cli.project_directory.as_deref(), &compose_files[0]);
	let project = resolve_project_name(cli.project, file.name.as_deref(), &base_dir);

	// Validate the resolved project name at the trust boundary, before it reaches
	// any code path that builds a filesystem path from it (staging, lock files,
	// quadlet generation). Explicit `-p`/`COMPOSE_PROJECT_NAME` values and the
	// compose `name:` field are otherwise taken verbatim; rejecting an unsafe
	// name here fails closed regardless of which command runs next.
	if !podup::is_safe_project_name(&project) {
		return Err(podup::ComposeError::Unsupported(format!(
			"project name {project:?} is not a safe path component: use only ASCII \
			 letters, digits, '-', '_', '.', not starting with '.', max 128 chars"
		)));
	}

	// `generate` produces declarative artifacts from the compose file alone; it
	// neither contacts Podman nor mutates project state.
	if let Commands::Generate {
		kind: GenerateCommands::Quadlet { output },
	} = &cli.command
	{
		return write_quadlet(&file, &project, output.as_deref());
	}

	let client = podup::podman::connect(cli.socket.as_deref())?;
	// The `-t/--timeout` shutdown-grace override applies to every command that
	// stops containers (up recreate, down, stop, restart).
	let stop_timeout = match &cli.command {
		Commands::Up { timeout, .. }
		| Commands::Down { timeout, .. }
		| Commands::Stop { timeout, .. }
		| Commands::Restart { timeout, .. } => *timeout,
		_ => None,
	};
	// `--scale SERVICE=N` (on `up`) and the `scale` subcommand both feed the
	// engine's replica overrides so `resolve_replicas` reports the target count.
	let scale_overrides: std::collections::HashMap<String, u32> = match &cli.command {
		Commands::Up { scale, .. } => scale.iter().cloned().collect(),
		Commands::Scale { pairs } => pairs.iter().cloned().collect(),
		_ => std::collections::HashMap::new(),
	};
	let engine = podup::Engine::with_base_dir(client, project, base_dir)
		.with_stop_timeout(stop_timeout)
		.with_scale_overrides(scale_overrides);

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
			timeout: _,
			scale: _,
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
		Commands::Down {
			volumes,
			timeout: _,
		} => engine.down_with_options(&file, volumes).await?,
		Commands::Start { services } => engine.start(&file, &services).await?,
		Commands::Stop {
			services,
			timeout: _,
		} => engine.stop(&file, &services).await?,
		Commands::Scale { pairs } => engine.scale(&file, &pairs).await?,
		Commands::Create {
			build,
			force_recreate,
			no_recreate,
			services,
		} => {
			if build {
				engine.build_all(&file, &services).await?;
			}
			engine
				.create_with_options(
					&file,
					&cli.profile,
					&services,
					no_recreate,
					force_recreate,
					false,
				)
				.await?
		}
		Commands::Build {
			no_cache,
			pull,
			build_arg,
			quiet,
			services,
		} => {
			engine
				.build_all_with_options(
					&file,
					&services,
					&podup::BuildOptions {
						no_cache,
						pull,
						build_args: build_arg,
						quiet,
					},
				)
				.await?
		}
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
		Commands::Ps { all, quiet, format } => {
			engine
				.ps_with_options(
					&file,
					podup::PsOptions {
						all,
						quiet,
						json: format == OutputFormat::Json,
					},
				)
				.await?
		}
		Commands::Top { services } => engine.top(&file, &services).await?,
		Commands::Stats {
			no_stream,
			services,
		} => engine.stats(&file, &services, no_stream).await?,
		Commands::Push {
			ignore_push_failures,
			tls_verify,
			services,
		} => {
			engine
				.push(
					&file,
					&services,
					podup::PushOptions {
						ignore_failures: ignore_push_failures,
						tls_verify,
					},
				)
				.await?
		}
		Commands::Port {
			service,
			private_port,
			proto,
		} => engine.port(&file, &service, private_port, &proto).await?,
		Commands::Images { quiet, format } => {
			engine
				.images_with_options(
					&file,
					podup::ImagesOptions {
						quiet,
						json: format == OutputFormat::Json,
					},
				)
				.await?
		}
		Commands::Logs {
			service,
			follow,
			tail,
			since,
			until,
			timestamps,
		} => {
			engine
				.logs_with_options(
					&file,
					service.as_deref(),
					podup::LogsOptions {
						follow,
						tail,
						since,
						until,
						timestamps,
					},
				)
				.await?
		}
		Commands::Exec {
			env,
			user,
			workdir,
			privileged,
			detach,
			no_tty: _,
			index,
			service,
			cmd,
		} => {
			engine
				.exec_with_options(
					&file,
					&service,
					cmd,
					podup::ExecOptions {
						env,
						user,
						workdir,
						privileged,
						detach,
						index,
					},
				)
				.await?
		}
		Commands::Pull { services } => engine.pull_services(&file, &services).await?,
		Commands::Restart {
			service,
			timeout: _,
		} => engine.restart(&file, service.as_deref()).await?,
		Commands::Config => unreachable!("handled above"),
		Commands::Generate { .. } => unreachable!("handled above"),
		Commands::Watch => engine.watch(&file).await?,
		Commands::Ls { .. } => unreachable!("handled before compose parsing"),
		#[cfg(feature = "update")]
		Commands::Update { .. } => unreachable!("handled before compose parsing"),
		#[cfg(feature = "completions")]
		Commands::Completions { .. } => unreachable!("handled before compose parsing"),
	}

	Ok(())
}
