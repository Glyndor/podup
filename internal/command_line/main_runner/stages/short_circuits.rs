//! Per-command arms that short-circuit before the engine-build path.
//!
//! Each helper here is the body of one command that `run` handles before the engine path:
//! the arm bodies that return before the resolve-compose / parse-file /
//! engine-build flow runs. Order, side effects, and early returns are
//! preserved verbatim from `main::run`.

use std::path::PathBuf;

use crate::cli::{AuditFormat, Cli, Commands, PsArgs};
use crate::startup::resolve::{resolve_base_dir, resolve_compose_files, resolve_project_name};
use crate::startup::validate_project_name;

use crate::audit;

use super::super::label_only::fail_on_named_file_missing;
use super::super::socket::connect_podman;
#[cfg(feature = "update")]
use super::super::update_globals::misused_update_global;

/// Pure early-return arms: `help`, `version`, `completions`, `update`.
///
/// Handle `help [COMMAND]` by printing the deepest-subcommand help (tolerating
/// `-h`/`--help` / `--`). Other commands in the file follow the same
/// "render-and-return" pattern.
pub(crate) fn run_help(commands: &[String]) -> podup::Result<()> {
	super::super::help::print_command_help(commands)
}

/// Handle `version [--short] [--format json]`.
pub(crate) fn run_version(short: bool, format: &str) -> podup::Result<()> {
	use std::io::Write;
	let version = env!("CARGO_PKG_VERSION");
	let line = if short {
		version.to_string()
	} else if format == "json" {
		format!("{{\"version\":\"v{version}\"}}")
	} else {
		format!("podup version v{version}")
	};
	match writeln!(std::io::stdout(), "{line}") {
		Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e.into()),
		_ => Ok(()),
	}
}

/// Handle `completions <shell>`. Gated by the `completions` feature.
#[cfg(feature = "completions")]
pub(crate) fn run_completions(shell: clap_complete::Shell) -> podup::Result<()> {
	use clap::CommandFactory;
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
		Ok(()) => Ok(()),
		Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
		Err(e) => Err(e.into()),
	}
}

/// Handle `update [--check|--force]`. Gated by the `update` feature.
#[cfg(feature = "update")]
pub(crate) async fn run_update(check: bool, force: bool) -> podup::Result<()> {
	// The compose-only global value-flags (--socket/--profile/--env-file/
	// --project-directory) never reach the binary-only self-update, so accepting
	// them silently is a misleading no-op. Reject any that were passed on the
	// command line (env-sourced values are left alone; they are not a misuse).
	if let Some(flag) = misused_update_global() {
		use clap::CommandFactory;
		Cli::command()
			.error(
				clap::error::ErrorKind::ArgumentConflict,
				format!(
					"`{flag}` has no effect on `update`, which replaces the podup \
						 binary itself rather than acting on a compose project"
				),
			)
			.exit();
	}
	let opts = podup::update::UpdateOptions {
		check_only: check,
		force,
	};
	tokio::task::spawn_blocking(move || podup::update::run(opts))
		.await
		.map_err(|e| podup::ComposeError::Update(format!("update task failed: {e}")))?
}

/// Pre-parse short-circuit arms (no compose file, but `--list-checks` /
/// `ls` / `ps` reach Podman).
///
/// Handle `audit --list-checks`. Renders the check registry directly without
/// reading a compose file.
pub(crate) fn render_audit_list_checks(format: AuditFormat) {
	match format {
		AuditFormat::Table => audit::render_list_checks_table(),
		AuditFormat::Json => audit::render_list_checks_json(),
	}
}

/// Handle `ls`. Connects to Podman without touching the compose file.
pub(crate) async fn run_ls(
	all: bool,
	quiet: bool,
	filter: &[String],
	format: crate::cli::OutputFormat,
	env_file: &[String],
	socket: Option<&str>,
	connection_pool_size: Option<usize>,
) -> podup::Result<()> {
	// `ls` is project-agnostic and short-circuits before compose parsing, so
	// validate the global `--env-file` paths it would otherwise silently
	// ignore (the other paths validate them while parsing).
	for ef in env_file {
		if !std::path::Path::new(ef).is_file() {
			return Err(podup::ComposeError::Unsupported(format!(
				"--env-file {ef} does not exist or is not a file"
			)));
		}
	}
	let client = connect_podman(socket, connection_pool_size).await?;
	podup::list_projects_filtered(
		&client,
		podup::LsOptions::new(all, quiet, format == crate::cli::OutputFormat::Json),
		filter,
	)
	.await
}

/// Handle `ps`. Reads the compose file when present and parseable, otherwise
/// falls back to an empty model so a project whose file is gone still answers.
pub(crate) async fn run_ps(
	cli: &Cli,
	error_context: &mut crate::startup::exit_status::ErrorContext,
) -> podup::Result<()> {
	let compose_files = resolve_compose_files(&cli.file);
	fail_on_named_file_missing(&cli.file, &compose_files)?;
	let base_dir = resolve_base_dir(cli.project_directory.as_deref(), &compose_files[0]);
	// `ps` is a read-only command that does not start containers, so the
	// parse-time "port ... is published on every interface" warning has
	// nothing to confirm: nothing on the host is being bound right now.
	// Suppress the warning for this command regardless of `--no-warn` so
	// operators reading `ps` in a loop are not warned on a port bind they
	// already accepted.
	let _suppress_port_warn = podup::compose::SuppressPortExposureGuard::new();
	// Parse the compose file when it is present and valid so `--services` and a
	// positional `SERVICE` filter can resolve service names and replicas;
	// otherwise fall back to an empty model (and the directory basename for the
	// project name) rather than failing.
	//
	// A file that is PRESENT but does not parse is a different case and is
	// propagated: the old `.ok()` swallowed it, so `podup ps` printed a bare
	// header and exited 0 on a broken compose file, which a health check or a
	// deploy gate reads as "nothing running, no error". `docker compose ps`
	// exits non-zero there, and so does every other podup command on the same
	// file.
	let parsed = match podup::parse_files_with_env_files(&compose_files, &cli.env_file) {
		Ok(f) => Some(f),
		Err(e) if compose_files.iter().any(|p| p.exists()) => return Err(e),
		Err(_) => None,
	};
	let compose_name = parsed.as_ref().and_then(|f| f.name.clone());
	let file = parsed.unwrap_or_default();
	let project = resolve_project_name(cli.project.clone(), compose_name.as_deref(), &base_dir);
	validate_project_name(&project)?;
	error_context.set_compose(&file, &project);
	// Identity colours key on the project-stripped label, so every command
	// tints the same container the same way.
	podup::ui::set_project(&project);
	// Register the service names so each gets its own identity colour. The
	// hash this replaces could not know how many services existed, so two
	// could share one. The `down`-by-label path has no compose file to read
	// service names from up front; it registers them itself, from the live
	// container listing it fetches immediately before tearing down.
	podup::ui::set_services(&file.services.keys().cloned().collect::<Vec<_>>());

	let Commands::Ps(PsArgs {
		all,
		quiet,
		services_only,
		size,
		filter,
		status,
		format,
		services,
	}) = &cli.command
	else {
		// The caller (top-level `run`) only routes here when the variant matches.
		return Ok(());
	};
	let client = connect_podman(cli.socket.as_deref(), cli.connection_pool_size).await?;
	let engine = podup::Engine::with_base_dir(client, project, base_dir);
	engine
		.ps_filtered_with_display(
			&file,
			podup::PsOptions::new(*all, *quiet, *format == crate::cli::OutputFormat::Json),
			podup::PsFilterOptions::new(
				*services_only,
				services.clone(),
				status.clone(),
				filter.clone(),
			),
			podup::PsDisplayOptions::default().with_size(*size),
		)
		.await
}

/// Handle `down -p NAME` when no compose file resolves: tear the project down
/// purely by its `podup.project` label, matching `docker compose -p NAME down`.
pub(crate) async fn run_down_by_label(cli: &Cli, compose_files: &[PathBuf]) -> podup::Result<()> {
	let Commands::Down {
		volumes,
		rmi,
		timeout,
		..
	} = &cli.command
	else {
		return Ok(());
	};
	let project = cli.project.clone().expect("checked by down_by_label_path");
	validate_project_name(&project)?;
	// Identity colours key on the project-stripped label, so every command
	// tints the same container the same way.
	podup::ui::set_project(&project);
	// `--rmi` removes the images of the file's services, which cannot be
	// resolved without a file; warn rather than silently dropping it.
	if rmi.is_some() {
		tracing::warn!(
			"--rmi has no effect without a compose file; containers, networks and \
			 volumes are still removed by project label"
		);
	}
	let base_dir = resolve_base_dir(cli.project_directory.as_deref(), &compose_files[0]);
	let stop_timeout = podup::validate_stop_timeout(*timeout)?;
	let client = connect_podman(cli.socket.as_deref(), cli.connection_pool_size).await?;
	let engine =
		podup::Engine::with_base_dir(client, project, base_dir).with_stop_timeout(stop_timeout);
	// `down` is mutating, so serialize it against concurrent runs as the
	// normal teardown path does.
	let _lock = engine.lock_project()?;
	engine.down_by_label(*volumes).await
}

/// Set the global UI/output state parsed from the CLI before any command runs:
/// colour choice, lifecycle progress on stderr, and the tracing floor.
pub(crate) fn setup_output(cli: &Cli) {
	// Resolve the colour choice before any output (including tracing setup below)
	// so `--ansi`/`NO_COLOR`/TTY detection apply consistently everywhere.
	podup::ui::set_color_choice(cli.ansi.into());
	// Enable user-facing lifecycle progress for the CLI: per-container
	// Started/Stopped/Removed lines on stderr (and the `run -d` id on stdout),
	// matching docker compose. The library leaves this off so embedders stay
	// silent. Only the lifecycle commands emit it, so enabling it globally here
	// is inert for read-only/machine-output commands.
	podup::ui::set_progress(true);
	// `watch` is an interactive, long-running command; surface its per-action
	// progress (synced/rebuilt/restarted) by defaulting to INFO instead of the
	// quiet WARN floor. `RUST_LOG` always overrides.
	let log_floor = if matches!(cli.command, Commands::Watch) {
		"info"
	} else {
		"warn"
	};
	crate::startup::init_tracing(log_floor);
}
