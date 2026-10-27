//! Per-command arms that short-circuit after the compose file has been parsed:
//! `config`, `audit`, `generate quadlet`, `autostart`, plus the engine-build
//! path that drives every other lifecycle command. Order, side effects, and
//! early returns are preserved verbatim from `main::run`.

use std::path::PathBuf;

use crate::cli::{AuditFormat, AutostartCommands, Cli, Commands, ConfigArgs, UpArgs};
use crate::startup::resolve::{absolute, resolve_base_dir, resolve_project_name};
use crate::startup::validate_project_name;

use crate::audit;

use super::super::socket::{connect_podman, resolve_socket};

/// Handle `config`. Returns the rendered project information without ever
/// contacting Podman unless `--resolve-image-digests` was set.
pub(crate) async fn run_config(
	cli: &Cli,
	compose_files: &[PathBuf],
	file: podup::compose::types::ComposeFile,
	error_context: &mut crate::startup::exit_status::ErrorContext,
) -> podup::Result<()> {
	let Commands::Config(ConfigArgs {
		format,
		services,
		volumes,
		images,
		profiles,
		hash,
		quiet,
		resolve_image_digests,
		..
	}) = &cli.command
	else {
		return Ok(());
	};
	// Validate the resolved project name in the config path too, at the same
	// trust boundary the mutating commands use, so `config -p 'bad name!'`
	// reports the same invalid-name error instead of succeeding. The config
	// path returns early below, so without this it would never be checked.
	let base_dir = resolve_base_dir(cli.project_directory.as_deref(), &compose_files[0]);
	let project = resolve_project_name(cli.project.clone(), file.name.as_deref(), &base_dir);
	validate_project_name(&project)?;
	error_context.set_compose(&file, &project);
	// Identity colours key on the project-stripped label, so every command
	// tints the same container the same way.
	podup::ui::set_project(&project);
	podup::ui::set_services(&file.services.keys().cloned().collect::<Vec<_>>());
	// `file` is already parsed with the correct interpolation setting above.
	let parsed = file;
	// `--resolve-image-digests` pins each image to its registry digest, which
	// needs a Podman connection to inspect images.
	let mut resolved = if *resolve_image_digests {
		let client = connect_podman(cli.socket.as_deref(), cli.connection_pool_size).await?;
		podup::resolve_image_digests(&client, &parsed).await?
	} else {
		parsed
	};
	// Honor active profiles so `config` prints the same services `up` starts.
	podup::retain_active_profiles(&mut resolved, &cli.profile);
	// Fold `env_file:` into `environment:` the way docker compose does, so a
	// service whose whole environment comes from a file does not render with no
	// environment at all (#1184). `config` is the command you reach for to ask
	// what will actually run; leaving the key unresolved pointed away from the
	// answer rather than merely omitting it.
	podup::env_file::materialize_env_files(&mut resolved, &base_dir)?;
	// Render the resolved project name (already settled above from -p /
	// COMPOSE_PROJECT_NAME, the top-level `name:`, then the directory
	// basename), like `docker compose config`, rather than echoing the file's
	// literal `name:`.
	crate::startup::render_config(
		&resolved,
		format,
		&crate::startup::ConfigOutput {
			services: *services,
			volumes: *volumes,
			images: *images,
			profiles: *profiles,
			hash: hash.clone(),
			quiet: *quiet,
		},
		&project,
		&base_dir,
	)
}

/// Handle `audit` (without `--list-checks`, which short-circuits earlier).
pub(crate) fn render_audit(
	cli: &Cli,
	compose_files: &[PathBuf],
	file: &podup::compose::types::ComposeFile,
	error_context: &mut crate::startup::exit_status::ErrorContext,
) -> podup::Result<()> {
	let Commands::Audit {
		strict,
		format,
		wildcard_binds,
		..
	} = &cli.command
	else {
		return Ok(());
	};
	let base_dir = resolve_base_dir(cli.project_directory.as_deref(), &compose_files[0]);
	let project = resolve_project_name(cli.project.clone(), file.name.as_deref(), &base_dir);
	validate_project_name(&project)?;
	error_context.set_compose(file, &project);
	podup::ui::set_project(&project);
	podup::ui::set_services(&file.services.keys().cloned().collect::<Vec<_>>());
	let mut resolved = file.clone();
	podup::retain_active_profiles(&mut resolved, &cli.profile);
	let containers = audit::prepare_containers(&resolved, &base_dir)?;
	// `--wildcard-binds` is opt-in; collect the enabled flags into the slice
	// the registry reads so a future opt-in check only needs a new
	// `opt_in: Some("--new-flag")` here (#1881).
	let mut opt_in: Vec<&'static str> = Vec::new();
	if *wildcard_binds {
		opt_in.push("--wildcard-binds");
	}
	let report = audit::audit_containers(&resolved, &containers, &opt_in);
	let output = match format {
		AuditFormat::Table => audit::render_table(&containers, &report),
		AuditFormat::Json => audit::render_json(&report),
	};
	if !audit::output_complete(output)? {
		return Ok(());
	}
	if *strict && report.has_findings() {
		std::process::exit(1);
	}
	Ok(())
}

/// Handle `generate quadlet [-o PATH]`.
pub(crate) fn run_generate(
	cli: &Cli,
	project: &str,
	base_dir: &std::path::Path,
	file: &podup::compose::types::ComposeFile,
	output: Option<&std::path::Path>,
) -> podup::Result<()> {
	// Honor active profiles so `generate quadlet` emits the same services
	// `up` would start, instead of also emitting units for inactive-profile
	// services (which would make `--profile` a no-op on this subcommand).
	let mut filtered = file.clone();
	podup::retain_active_profiles(&mut filtered, &cli.profile);
	// Absolute base dir so a `.build` unit's context resolves under the compose
	// file, not the unit directory the systemd generator would otherwise use.
	let canonical_base_dir =
		std::fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
	crate::main_runner::generate::write_quadlet(
		&filtered,
		project,
		&canonical_base_dir,
		output,
		cli.no_warn,
	)
}

/// Handle `autostart`. `install` and `status` work from the compose file alone
/// and never contact Podman, except `uninstall --purge`, which connects via
/// the per-project env below.
pub(crate) async fn run_autostart(
	cli: &Cli,
	compose_files: &[PathBuf],
	project: String,
	base_dir: PathBuf,
	file: &podup::compose::types::ComposeFile,
	kind: &AutostartCommands,
) -> podup::Result<()> {
	let env = crate::main_runner::autostart::AutostartEnv {
		profile: &cli.profile,
		env_files: &cli.env_file,
		socket: resolve_socket(cli.socket.as_deref()),
		connection_pool_size: cli.connection_pool_size,
	};
	crate::main_runner::autostart::dispatch(&env, compose_files, project, base_dir, file, kind)
		.await
}

/// Build the engine, project-name guards, and project lock for the lifecycle
/// commands, then hand off to `dispatch::dispatch`. Reached by every arm not
/// matched by an earlier short-circuit.
pub(crate) async fn run_engine_command(
	cli: Cli,
	compose_files: &[PathBuf],
	file: &podup::compose::types::ComposeFile,
	project: String,
	base_dir: PathBuf,
) -> podup::Result<()> {
	let client = connect_podman(cli.socket.as_deref(), cli.connection_pool_size).await?;
	// The `-t/--timeout` shutdown-grace override applies to every command that
	// stops containers (up recreate, down, stop, restart).
	let stop_timeout = match &cli.command {
		Commands::Up(UpArgs { timeout, .. })
		| Commands::Down { timeout, .. }
		| Commands::Stop { timeout, .. }
		| Commands::Restart { timeout, .. } => *timeout,
		_ => None,
	};
	// Reject a `-t/--timeout` below -1 here, at the trust boundary, with a clear
	// message instead of forwarding it to libpod as a raw `?t=<negative>` 400.
	let stop_timeout = podup::validate_stop_timeout(stop_timeout)?;
	// `--scale SERVICE=N` (on `up`) and the `scale` subcommand both feed the
	// engine's replica overrides so `resolve_replicas` reports the target count.
	let scale_overrides: std::collections::HashMap<String, u32> = match &cli.command {
		Commands::Up(UpArgs { scale, .. }) => scale.iter().cloned().collect(),
		Commands::Scale { pairs } => pairs.iter().cloned().collect(),
		_ => std::collections::HashMap::new(),
	};
	// `up` image-acquisition overrides: `--pull`, `--no-build`, `--quiet-pull`.
	let (pull_override, no_build, quiet_pull) = match &cli.command {
		Commands::Up(UpArgs {
			pull,
			no_build,
			quiet_pull,
			..
		}) => (pull.clone(), *no_build, *quiet_pull),
		Commands::Pull { quiet, policy, .. } => (policy.clone(), false, *quiet),
		Commands::Create { pull, .. } => (pull.clone(), false, false),
		_ => (None, false, false),
	};
	// `up -V/--renew-anon-volumes`: recreate anonymous volumes on container
	// recreation instead of leaving the old ones orphaned.
	let renew_anon_volumes = matches!(
		&cli.command,
		Commands::Up(UpArgs {
			renew_anon_volumes: true,
			..
		})
	);
	let engine = podup::Engine::with_base_dir(client, project, base_dir)
		.with_compose_files(compose_files.iter().map(|f| absolute(f)).collect())
		.with_stop_timeout(stop_timeout)
		.with_scale_overrides(scale_overrides)
		.with_up_overrides(pull_override, no_build, quiet_pull)
		.with_run_overrides(crate::startup::run_overrides_for(&cli.command))
		.with_run_env_files(cli.env_file.clone())
		.with_run_labels(crate::startup::run_labels_for(&cli.command))
		.with_run_no_tty(crate::startup::run_no_tty_for(&cli.command))
		.with_renew_anon_volumes(renew_anon_volumes)
		.with_no_warn(cli.no_warn);

	// Serialize mutating lifecycle commands against concurrent `podup` runs on
	// the same project. Read-only / follow commands (ps, logs, top, port,
	// images, exec, pull, cp, config, watch) take no lock so they don't block
	// or get blocked. The guard is held until `run` returns.
	let _lock = if crate::startup::is_mutating(&cli.command) {
		Some(engine.lock_project()?)
	} else {
		None
	};

	crate::dispatch::dispatch(&engine, file, cli.command, &cli.profile).await
}
