//! `podup` — docker-compose to Podman translator CLI.

// The binary carries no `unsafe`; deny it so any future addition is caught.
#![deny(unsafe_code)]

use std::process;

#[cfg(feature = "completions")]
use clap::CommandFactory;
use clap::Parser;
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::EnvFilter;

mod cli;
mod generate;
mod resolve;

use cli::*;
use generate::write_quadlet;
use resolve::*;

/// Canonical project URL, reused for the bug-report hint on internal errors.
const REPO_URL: &str = "https://github.com/Glyndor/podup";

/// Event formatter that renders every diagnostic as `podup: <level>: <message>`
/// on a single line, matching the prefix used by the CLI's own `eprintln!`
/// warnings and errors. This unifies the compose forward-compat diagnostics
/// (emitted via `tracing::warn!`) with the rest of podup's user-facing output.
struct PodupFormat;

impl<S, N> FormatEvent<S, N> for PodupFormat
where
	S: Subscriber + for<'a> LookupSpan<'a>,
	N: for<'a> FormatFields<'a> + 'static,
{
	fn format_event(
		&self,
		ctx: &FmtContext<'_, S, N>,
		mut writer: Writer<'_>,
		event: &Event<'_>,
	) -> std::fmt::Result {
		write!(writer, "podup: {}: ", level_word(*event.metadata().level()))?;
		ctx.field_format().format_fields(writer.by_ref(), event)?;
		writeln!(writer)
	}
}

/// Map a tracing level to the user-facing word used in `podup:` output.
fn level_word(level: tracing::Level) -> &'static str {
	match level {
		tracing::Level::ERROR => "error",
		tracing::Level::WARN => "warning",
		tracing::Level::INFO => "info",
		tracing::Level::DEBUG => "debug",
		tracing::Level::TRACE => "trace",
	}
}

/// Guidance printed after an internal error or panic: where to report it and a
/// reminder to scrub secrets first. Kept off ordinary, user-correctable errors.
fn internal_error_notice() -> String {
	format!(
		"podup: this looks like a bug; re-run with RUST_LOG=debug and report it at {REPO_URL}/issues\n\
		 podup: redact secrets (passwords, tokens, resolved env values) from any logs before sharing"
	)
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
			| Commands::Scale { .. }
			| Commands::Create { .. }
	)
}

#[tokio::main]
async fn main() {
	// Replace the default panic output (a raw Rust backtrace) with a `podup:`
	// internal-error notice that tells the user what to report and where, plus
	// the reminder to redact secrets first.
	std::panic::set_hook(Box::new(|info| {
		eprintln!("podup: internal error: {info}");
		eprintln!("{}", internal_error_notice());
	}));

	match run().await {
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
	// Surface diagnostics by default: with no `RUST_LOG` set, show warnings (so
	// the forward-compat "unknown field" notices are never silently dropped),
	// and write them to stderr so a command's stdout stays a clean pipe.
	tracing_subscriber::fmt()
		.with_env_filter(
			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
		)
		.with_writer(std::io::stderr)
		.event_format(PodupFormat)
		.init();

	// Frame `--help`/`--version` output with a blank line top and bottom. clap
	// trims template/before/after-help edges, so wrap the rendered text here.
	let cli = match Cli::try_parse() {
		Ok(cli) => cli,
		Err(e) => match e.kind() {
			clap::error::ErrorKind::DisplayHelp
			| clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
			| clap::error::ErrorKind::DisplayVersion => {
				print!("\n{e}\n");
				process::exit(0);
			}
			_ => e.exit(),
		},
	};

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
		Commands::Logs { service, follow } => {
			engine.logs(&file, service.as_deref(), follow).await?
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn level_words_match_user_facing_terms() {
		assert_eq!(level_word(tracing::Level::WARN), "warning");
		assert_eq!(level_word(tracing::Level::ERROR), "error");
	}

	#[test]
	fn internal_error_notice_reports_and_warns_on_secrets() {
		let notice = internal_error_notice();
		assert!(notice.contains(REPO_URL), "points at the issue tracker");
		assert!(notice.contains("/issues"));
		assert!(
			notice.contains("redact"),
			"reminds the user to scrub secrets"
		);
		assert!(
			notice.contains("RUST_LOG=debug"),
			"tells the user what to capture"
		);
	}
}
