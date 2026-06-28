//! `podup` — docker-compose to Podman translator CLI.

// The binary carries no `unsafe`; deny it so any future addition is caught.
#![deny(unsafe_code)]

use std::process;

#[cfg(feature = "completions")]
use clap::CommandFactory;

mod cli;
mod dispatch;
mod generate;
mod resolve;
mod startup;

use cli::*;
use generate::write_quadlet;
use resolve::*;
use startup::{init_tracing, internal_error_notice, is_mutating, parse_cli};

fn main() {
	// Replace the default panic output (a raw Rust backtrace) with a `podup:`
	// internal-error notice that tells the user what to report and where, plus
	// the reminder to redact secrets first.
	std::panic::set_hook(Box::new(|info| {
		// A broken pipe (a downstream reader closed early, e.g. `podup ls | head`)
		// surfaces as a panic from the failing `println!`/`eprintln!` because Rust
		// ignores SIGPIPE. With `panic = "abort"` that would escalate to SIGABRT
		// (exit 134) and a misleading bug-report notice; instead exit quietly with
		// success like any well-behaved Unix tool.
		if startup::is_broken_pipe_panic(&info.to_string()) {
			std::process::exit(0);
		}
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
			print_error(&e);
			process::exit(podup::update::exit_code(&e));
		}
		Err(e) => {
			print_error(&e);
			process::exit(1);
		}
	}
}

/// Print a top-level error to stderr with a colour-aware bold-red `error:` label.
/// anstream strips the styling when stderr is not a terminal or colour is off.
fn print_error(e: &podup::ComposeError) {
	use std::io::Write;
	let style = podup::ui::error_style();
	let mut err = anstream::stderr();
	let _ = writeln!(
		err,
		"podup: {}error:{} {e}",
		style.render(),
		style.render_reset()
	);
}

/// Orchestrate one CLI invocation: parse args, then short-circuit the commands
/// that need neither a compose file nor (for some) Podman — `completions`,
/// `update`, `ls`, and `config`. Otherwise resolve and parse the compose
/// file(s), settle the project name and base directory (validating the name at
/// the trust boundary), acquire the per-project lock, and dispatch the
/// remaining commands.
async fn run() -> podup::Result<()> {
	let cli = parse_cli();
	// Resolve the colour choice before any output (including tracing setup below)
	// so `--ansi`/`NO_COLOR`/TTY detection apply consistently everywhere.
	podup::ui::set_color_choice(cli.ansi.into());
	// `watch` is an interactive, long-running command; surface its per-action
	// progress (synced/rebuilt/restarted) by defaulting to INFO instead of the
	// quiet WARN floor. `RUST_LOG` always overrides.
	let log_floor = if matches!(cli.command, Commands::Watch) {
		"info"
	} else {
		"warn"
	};
	init_tracing(log_floor);

	// `completions` derives entirely from the static CLI definition; it neither
	// parses a compose file nor contacts Podman. Print to stdout for piping.
	#[cfg(feature = "completions")]
	if let Commands::Completions { shell } = cli.command {
		let mut cmd = Cli::command();
		let name = cmd.get_name().to_string();
		// Render into a buffer first: clap_complete panics if the writer errors,
		// which aborts (SIGABRT) when stdout is a closed pipe such as
		// `podup completions bash | head`. A `Vec` write never fails; then send
		// it to stdout and treat a broken pipe as a clean exit, like a normal
		// Unix tool.
		let mut buf = Vec::new();
		clap_complete::generate(shell, &mut cmd, name, &mut buf);
		match std::io::Write::write_all(&mut std::io::stdout(), &buf) {
			Ok(()) => return Ok(()),
			Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
			Err(e) => return Err(e.into()),
		}
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

	if let Commands::Config {
		format,
		services,
		quiet,
		no_interpolate,
		resolve_image_digests,
	} = &cli.command
	{
		// `--no-interpolate` re-parses with substitution disabled; `file` (already
		// parsed with interpolation) is used otherwise.
		let parsed = if *no_interpolate {
			podup::parse_files_with_env_files_interp(&compose_files, &cli.env_file, false)?
		} else {
			file
		};
		// `--resolve-image-digests` pins each image to its registry digest, which
		// needs a Podman connection to inspect images.
		let mut resolved = if *resolve_image_digests {
			let client = podup::podman::connect(cli.socket.as_deref())?;
			podup::resolve_image_digests(&client, &parsed).await?
		} else {
			parsed
		};
		// Honor active profiles so `config` prints the same services `up` starts.
		podup::retain_active_profiles(&mut resolved, &cli.profile);
		// Resolve the effective project name (-p / COMPOSE_PROJECT_NAME, then the
		// top-level `name:`, then the directory basename) and render it, like
		// `docker compose config` — rather than echoing the file's literal `name:`.
		let base_dir = resolve_base_dir(cli.project_directory.as_deref(), &compose_files[0]);
		let project =
			resolve_project_name(cli.project.clone(), resolved.name.as_deref(), &base_dir);
		return startup::render_config(&resolved, format, *services, *quiet, &project);
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
	// `up` image-acquisition overrides: `--pull`, `--no-build`, `--quiet-pull`.
	let (pull_override, no_build, quiet_pull) = match &cli.command {
		Commands::Up {
			pull,
			no_build,
			quiet_pull,
			..
		} => (pull.clone(), *no_build, *quiet_pull),
		Commands::Pull { quiet, policy, .. } => (policy.clone(), false, *quiet),
		_ => (None, false, false),
	};
	// `up -V/--renew-anon-volumes`: recreate anonymous volumes on container
	// recreation instead of leaving the old ones orphaned.
	let renew_anon_volumes = matches!(
		&cli.command,
		Commands::Up {
			renew_anon_volumes: true,
			..
		}
	);
	let engine = podup::Engine::with_base_dir(client, project, base_dir)
		.with_stop_timeout(stop_timeout)
		.with_scale_overrides(scale_overrides)
		.with_up_overrides(pull_override, no_build, quiet_pull)
		.with_run_overrides(startup::run_overrides_for(&cli.command))
		.with_renew_anon_volumes(renew_anon_volumes);

	// Serialize mutating lifecycle commands against concurrent `podup` runs on
	// the same project. Read-only / follow commands (ps, logs, top, port,
	// images, exec, pull, cp, config, watch) take no lock so they don't block
	// or get blocked. The guard is held until `run` returns.
	let _lock = if is_mutating(&cli.command) {
		Some(engine.lock_project()?)
	} else {
		None
	};

	dispatch::dispatch(&engine, &file, cli.command, &cli.profile).await
}
