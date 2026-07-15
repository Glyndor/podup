//! The `autostart` command group: manage a rootless `systemctl --user` unit that
//! brings the compose stack up at boot. Split out of `main::run` so that function
//! stays within the source line limit; `install` and `status` work from the
//! compose file alone and never contact Podman, while `uninstall --purge` is the
//! only branch that connects (to run the `down -v` teardown).

use std::path::PathBuf;

use podup::compose::types::ComposeFile;
use podup::ComposeError;

use crate::cli::{AutostartCommands, AutostartMode};

/// The slice of CLI globals the `autostart` dispatch needs, gathered up so the
/// already-consumed `Cli` (its `project` is moved earlier) need not be borrowed
/// whole.
pub(crate) struct AutostartEnv<'a> {
	pub profile: &'a [String],
	pub env_files: &'a [String],
	pub socket: Option<String>,
}

/// Handle the `autostart` command group. `install` and `status` never contact
/// Podman; `uninstall --purge` is the only branch that connects, to run the
/// `down -v` teardown.
pub(crate) async fn dispatch(
	env: &AutostartEnv<'_>,
	compose_files: &[PathBuf],
	project: String,
	base_dir: PathBuf,
	file: &ComposeFile,
	kind: &AutostartCommands,
) -> podup::Result<()> {
	match kind {
		AutostartCommands::Install {
			mode,
			no_start,
			dry_run,
		} => {
			// Only service mode is implemented; quadlet mode is honest about it.
			if *mode == AutostartMode::Quadlet {
				return Err(ComposeError::Autostart(
					"autostart --mode quadlet is not yet implemented (see #993); \
					 use the default --mode service"
						.to_string(),
				));
			}
			// systemd has no relative-path context, so resolve the exe, every compose
			// file, and the working directory to absolute paths the unit can embed.
			let exe = std::env::current_exe().map_err(|e| {
				ComposeError::Autostart(format!("cannot locate the podup executable: {e}"))
			})?;
			let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
			let mut abs_files = Vec::with_capacity(compose_files.len());
			for f in compose_files {
				abs_files.push(std::fs::canonicalize(f).map_err(|e| {
					ComposeError::Autostart(format!(
						"cannot resolve compose file {}: {e}",
						f.display()
					))
				})?);
			}
			let working_dir = std::fs::canonicalize(&base_dir).unwrap_or(base_dir);
			let opts = podup::autostart::InstallOptions {
				unit: podup::autostart::ServiceUnitOpts {
					exe,
					compose_files: abs_files,
					project,
					working_dir,
					profiles: env.profile.to_vec(),
					env_files: env.env_files.to_vec(),
				},
				no_start: *no_start,
				dry_run: *dry_run,
			};
			podup::autostart::install(&podup::autostart::RealSystemCtl, &opts)
		}
		AutostartCommands::Uninstall { purge } => {
			podup::autostart::uninstall(&podup::autostart::RealSystemCtl, &project)?;
			if *purge {
				// `--purge` is the only autostart branch that touches Podman: tear the
				// stack down and remove its named volumes via the normal `down -v` path.
				let client = podup::podman::connect(env.socket.as_deref())?;
				let engine = podup::Engine::with_base_dir(client, project, base_dir);
				let _lock = engine.lock_project()?;
				engine.down_with_options(file, true).await?;
			}
			Ok(())
		}
		AutostartCommands::Status => {
			podup::autostart::status(&podup::autostart::RealSystemCtl, &project)
		}
	}
}
