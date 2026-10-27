//! `podup` is the docker-compose to Podman translator CLI.

// The binary carries no `unsafe`; deny it so any future addition is caught.
#![deny(unsafe_code)]

use std::process;

// The binary's modules live in one folder; they are imported here under their
// own names so no path inside them depends on that folder.
mod command_line;
use command_line::audit;
use command_line::cli;
use command_line::dispatch;
use command_line::main_runner;
use command_line::startup;

use cli::*;
use startup::exit_status::{command_failure_exit_code, interrupt_exit_code, print_error};
use startup::resolve::{
	resolve_base_dir, resolve_compose_files, resolve_project_name, validate_project_directory,
};
use startup::{internal_error_notice, is_label_only, parse_cli, validate_project_name};

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
		// Match the panic PAYLOAD, not `info.to_string()`: the latter also carries
		// the source location, widening a substring search to text the panic
		// itself never chose.
		let payload = info.payload();
		let message = payload
			.downcast_ref::<&str>()
			.copied()
			.or_else(|| payload.downcast_ref::<String>().map(String::as_str))
			.unwrap_or_default();
		if startup::is_broken_pipe_panic(message) {
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
	let mut error_context = startup::exit_status::ErrorContext::default();
	let outcome = runtime.block_on(run(&mut error_context));
	// Shut the runtime down without waiting for its blocking pool. An
	// interactive exec/run always leaves one stdin read in flight there
	// (tokio's stdin reads on a blocking thread), and a plain drop waits for
	// it, so a session that ended with code 0 hung until the next keypress,
	// while any other code left through `process::exit` below and never
	// noticed (#1161). Everything user-visible is written and flushed by the
	// time `block_on` returns.
	runtime.shutdown_background();
	match outcome {
		Ok(()) => {}
		Err(podup::ComposeError::RunExited(code)) => process::exit(code as i32),
		// 128 + SIGINT, the shell convention, and what `docker compose up` returns
		// for SIGTERM as well as SIGINT. Nothing is printed: the operator pressed
		// Ctrl-C and knows it, and a compose file's own teardown output has
		// already gone to the terminal.
		Err(podup::ComposeError::Interrupted) => process::exit(interrupt_exit_code()),
		#[cfg(feature = "update")]
		Err(e @ podup::ComposeError::Update(_)) => {
			print_error(&e, &error_context);
			process::exit(podup::update::exit_code(&e));
		}
		Err(e) => {
			print_error(&e, &error_context);
			// A `run`/`exec` whose command cannot be launched arrives as a Podman
			// (OCI/crun) error; map it onto docker's conventional codes (127 not
			// found, 126 not executable) instead of the generic exit 1. Peel a
			// `DependencyNotReady` wrapper first so a failed readiness wait maps on
			// its real cause, exactly as an unwrapped one would.
			let code = match e.innermost() {
				podup::ComposeError::Podman(_) => command_failure_exit_code(&e.to_string()),
				_ => 1,
			};
			process::exit(code);
		}
	}
}

/// Orchestrate one CLI invocation: parse args, then short-circuit the commands
/// that need neither a compose file nor (for some) Podman. Otherwise resolve
/// and parse the compose file, settle the project name and base directory
/// (validating the name at the trust boundary), acquire the per-project lock,
/// and dispatch the remaining commands.
///
/// The per-command bodies live in [`main_runner::short_circuits`] and
/// [`main_runner::post_parse`]; the prelude below stays here because it runs in
/// strict order between commands and cannot be split.
async fn run(error_context: &mut startup::exit_status::ErrorContext) -> podup::Result<()> {
	let cli = parse_cli();
	main_runner::short_circuits::setup_output(&cli);

	// `help [COMMAND]` is served from the static CLI definition. Unlike clap's
	// built-in help subcommand it tolerates extra tokens, `-h`/`--help`, and a
	// leading `--`, and never errors with "unrecognized subcommand".
	if let Commands::Help { commands } = &cli.command {
		return main_runner::short_circuits::run_help(commands);
	}

	// `version` mirrors `docker compose version` (scripts probe it); it needs
	// neither a compose file nor Podman.
	if let Commands::Version { short, ref format } = cli.command {
		return main_runner::short_circuits::run_version(short, format);
	}

	// `completions` derives entirely from the static CLI definition; it neither
	// parses a compose file nor contacts Podman. Print to stdout for piping.
	#[cfg(feature = "completions")]
	if let Commands::Completions { shell } = cli.command {
		return main_runner::short_circuits::run_completions(shell);
	}

	// `update` operates on the binary itself, not a compose project, so it runs
	// before any compose file is parsed or Podman is contacted. The network and
	// filesystem work is blocking; keep it off the async path entirely.
	#[cfg(feature = "update")]
	if let Commands::Update { check, force } = cli.command {
		return main_runner::short_circuits::run_update(check, force).await;
	}

	// An explicit `--project-directory` must exist and be a directory before any
	// command relies on it (staging, lock files, quadlet output, base-dir
	// resolution); validate it once, at the trust boundary, for every command.
	validate_project_directory(cli.project_directory.as_deref())?;

	// `audit --list-checks` enumerates the check registry; no compose file or
	// Podman is needed. Short-circuit after project-directory validation but
	// before compose-file resolution, so an integrator can run it in any
	// directory with no `-f`. `--strict` already conflicts with
	// `--list-checks` at parse time, so this branch only sees a clean call.
	if let Commands::Audit {
		format,
		list_checks: true,
		..
	} = &cli.command
	{
		return main_runner::short_circuits::render_audit_list_checks(*format);
	}

	// `ls` discovers projects across the host by container label; it needs a
	// Podman connection but no compose file, so handle it before parsing one.
	if let Commands::Ls {
		all,
		quiet,
		filter,
		format,
	} = &cli.command
	{
		return main_runner::short_circuits::run_ls(
			*all,
			*quiet,
			filter,
			*format,
			&cli.env_file,
			cli.socket.as_deref(),
			cli.connection_pool_size,
		)
		.await;
	}

	// `ps` filters running containers purely by the project label and ignores the
	// compose file entirely, so it must list an already-running project even when
	// that file is missing or unparseable (matching `docker compose ps`). Handle
	// it before the compose file is parsed: read the compose `name:` for project
	// precedence when the file parses, but fall back to the directory basename
	// rather than failing, unlike the commands that depend on the file's contents.
	if let Commands::Ps(_) = &cli.command {
		return main_runner::short_circuits::run_ps(&cli, error_context).await;
	}

	let compose_files = resolve_compose_files(&cli.file);

	// `down -p NAME` must tear a running project down purely from its
	// `podup.project` label when no compose file is present, matching `docker
	// compose -p NAME down`. The startup flow otherwise parses the compose file
	// before dispatch, so a label-only teardown by project name fails on the
	// missing file. Handle it here, before that parse, but only when an explicit
	// project name was given and no file resolves, so a stray `down` in an empty
	// directory still errors rather than guessing a project from the basename.
	if main_runner::down_by_label::down_by_label_path(
		&cli.command,
		cli.project.as_deref(),
		compose_files.iter().any(|p| p.is_file()),
	) {
		return main_runner::short_circuits::run_down_by_label(&cli, &compose_files).await;
	}

	// `config --no-interpolate` must skip interpolation *entirely*: parsing with
	// interpolation enabled would evaluate a required-var `${VAR:?msg}` and fail
	// before we ever reached the no-interpolate branch. Detect it up front so the
	// file is parsed only once, with interpolation disabled.
	let no_interpolate = matches!(
		&cli.command,
		Commands::Config(ConfigArgs {
			no_interpolate: true,
			..
		})
	);
	// `events` and `ps` are scoped purely by the `podup.project` label and never
	// read service definitions, so, like `docker compose -p NAME events`/`ps`,
	// they must work against a running project even when no compose file is
	// present. Tolerate a missing file for these label-only commands by falling
	// back to an empty compose model; any other parse error (a malformed file
	// that *does* exist, a missing env file) still surfaces.
	let label_only = is_label_only(&cli.command);
	if label_only {
		main_runner::label_only::fail_on_named_file_missing(&cli.file, &compose_files)?;
	}
	// Show the port-exposure warning on `config` regardless of `--no-warn`,
	// and on `up`/`create`/`run`/`scale`/`watch`/`generate quadlet`/`autostart`
	// unless `--no-warn` is set. It can change a decision only where a container
	// is created or installed with the port. Suppress it on every other command:
	// those reuse containers whose configuration was already shown.
	let suppress_port_warn = match &cli.command {
		Commands::Config(_) => false,
		Commands::Up(_) | Commands::Create { .. } | Commands::Run(_) => cli.no_warn,
		Commands::Scale { .. } | Commands::Watch => cli.no_warn,
		Commands::Generate { .. } | Commands::Autostart { .. } => cli.no_warn,
		_ => true,
	};
	let _suppress_port_warn_guard =
		suppress_port_warn.then(podup::compose::SuppressPortExposureGuard::new);
	// The client-address warning (#1994) fires on almost any project with
	// published ports, so it is shown only where it can change a decision: the
	// commands that create containers, and `config`, which ignores `--no-warn`
	// as it does for the port-exposure warning.
	let show_client_address = match &cli.command {
		Commands::Config(_) => true,
		Commands::Up(_) | Commands::Create { .. } | Commands::Run(_) => !cli.no_warn,
		_ => false,
	};
	let _client_address_guard =
		show_client_address.then(podup::compose::ShowClientAddressWarningGuard::new);
	let file = if label_only && !compose_files.iter().any(|p| p.is_file()) {
		podup::compose::types::ComposeFile::default()
	} else {
		podup::parse_files_with_env_files_interp(&compose_files, &cli.env_file, !no_interpolate)?
	};

	if let Commands::Config(_) = &cli.command {
		return main_runner::post_parse::run_config(&cli, &compose_files, file, error_context)
			.await;
	}

	// `audit` is the read-only hardening view. It reuses the `config`
	// loading path verbatim: same parsed file, same active-profile filter,
	// same `env_file:` fold. It does not contact Podman. Honor active
	// profiles so a service left out by `--profile` does not get audited;
	// `audit` reports what `up` would start. `--list-checks` short-circuits
	// earlier, before compose-file resolution, so an integrator can run it
	// without a compose file.
	if let Commands::Audit { .. } = &cli.command {
		return main_runner::post_parse::render_audit(&cli, &compose_files, &file, error_context);
	}

	let base_dir = resolve_base_dir(cli.project_directory.as_deref(), &compose_files[0]);
	let project = resolve_project_name(cli.project.clone(), file.name.as_deref(), &base_dir);

	// Validate the resolved project name at the trust boundary, before it reaches
	// any code path that builds a filesystem path from it (staging, lock files,
	// quadlet generation). Explicit `-p`/`COMPOSE_PROJECT_NAME` values and the
	// compose `name:` field are otherwise taken verbatim; rejecting an unsafe
	// name here fails closed regardless of which command runs next.
	validate_project_name(&project)?;
	error_context.set_compose(&file, &project);
	// Identity colours key on the project-stripped label, so ps, logs, stats and
	// the progress lines all tint the same container the same way. This is the
	// path every lifecycle command takes.
	podup::ui::set_project(&project);
	podup::ui::set_services(&file.services.keys().cloned().collect::<Vec<_>>());

	// `generate` produces declarative artifacts from the compose file alone; it
	// neither contacts Podman nor mutates project state.
	if let Commands::Generate {
		kind: GenerateCommands::Quadlet { ref output },
	} = &cli.command
	{
		return main_runner::post_parse::run_generate(
			&cli,
			&project,
			&base_dir,
			&file,
			output.as_deref(),
		);
	}

	// `autostart` manages a rootless `systemctl --user` unit that brings the stack
	// up at boot. Like `generate` it works from the compose file alone and never
	// contacts Podman, except `uninstall --purge`, which tears the stack's volumes
	// down and so connects only in that branch.
	if let Commands::Autostart { kind } = &cli.command {
		return main_runner::post_parse::run_autostart(
			&cli,
			&compose_files,
			project,
			base_dir,
			&file,
			kind,
		)
		.await;
	}

	main_runner::post_parse::run_engine_command(cli, &compose_files, &file, project, base_dir).await
}
