//! The install / uninstall flow for service- and start-mode autostart.
//!
//! Both installers funnel through [`place_unit`], so the on-disk location and
//! the systemctl calls that follow it stay identical across modes; the
//! modes themselves only differ in which renderer produces the unit text and
//! which conflict check runs first. The auto-update timer pair lives in
//! [`place_update_units`] and installs/cleans up together with the main unit.
//!
//! Refusals (quadlet present, another single-unit mode present, the timer
//! install shadows the main one) live in [`conflict`]; this module only
//! calls them and proceeds.

use crate::ComposeError;

pub(super) mod conflict;
pub(super) mod detect;
pub(super) mod options;

use super::guards::emit_guards;
use super::paths::{unit_dir, unit_file_name, unit_path};
use super::service::{
	render_service_unit, render_update_service_unit, render_update_timer_unit,
	update_service_file_name, update_timer_file_name,
};
use super::start::{render_start_unit, validate_start_unit_opts, StartUnitOpts};
use super::systemctl::{checked, SystemCtl};
use options::InstallOptions;

/// Install (and, unless `no_start`, enable + start) the service-mode autostart
/// unit. Writes only under `${XDG_CONFIG_HOME:-~/.config}/systemd/user/`.
///
/// With `opts.auto_update_interval` set, also installs the sibling
/// `<unit>-update.service` (oneshot that runs `podup up -d`) and
/// `<unit>-update.timer` (the schedule that fires it). Removal takes all
/// three down together, see [`uninstall`].
pub fn install<S: SystemCtl>(sc: &S, opts: &InstallOptions) -> crate::Result<()> {
	let project = &opts.unit.project;
	conflict::refuse_if_quadlet_present(project)?;
	conflict::refuse_if_other_single_unit_mode(project, conflict::SingleUnitMode::Service)?;

	// Fail closed on values a unit line cannot represent (control characters
	// would inject directives via the literal `WorkingDirectory=` line).
	super::service::validate_unit_opts(&opts.unit).map_err(ComposeError::Autostart)?;

	if let Some(interval) = &opts.auto_update_interval {
		super::service::validate_auto_update_interval(interval).map_err(ComposeError::Autostart)?;
	}

	let unit_text = render_service_unit(&opts.unit);
	place_unit(sc, project, &unit_text, opts.dry_run, opts.no_start)?;

	if let Some(interval) = &opts.auto_update_interval {
		place_update_units(
			sc,
			project,
			&opts.unit,
			interval,
			opts.dry_run,
			opts.no_start,
		)?;
	}

	Ok(())
}

/// Install the start-mode unit: `ExecStart=podman start <container>`, so the
/// boot path resumes what Podman already holds instead of reconciling it
/// against the compose file. Single-service projects only; `sole_container`
/// is what refuses the rest, and its refusal names the mode to use instead.
pub fn install_start<S: SystemCtl>(
	sc: &S,
	opts: &StartUnitOpts,
	dry_run: bool,
	no_start: bool,
) -> crate::Result<()> {
	let project = &opts.project;
	conflict::refuse_if_quadlet_present(project)?;
	conflict::refuse_if_other_single_unit_mode(project, conflict::SingleUnitMode::Start)?;
	validate_start_unit_opts(opts).map_err(ComposeError::Autostart)?;
	let unit_text = render_start_unit(opts);
	place_unit(sc, project, &unit_text, dry_run, no_start)
}

/// Write the unit, reload systemd and (unless `no_start`) enable it. Shared by
/// every mode that writes one final `.service` file, so the two do not drift on
/// where the file lands or which systemctl calls follow it.
fn place_unit<S: SystemCtl>(
	sc: &S,
	project: &str,
	unit_text: &str,
	dry_run: bool,
	no_start: bool,
) -> crate::Result<()> {
	let path = unit_path(project);
	let unit_name = unit_file_name(project);

	// Surface the linger / session guards before acting (or previewing).
	emit_guards(sc);

	if dry_run {
		print!("{unit_text}");
		println!("\n# would write {}", path.display());
		println!("# would run: systemctl --user daemon-reload");
		if no_start {
			println!("# (--no-start) would not enable or start the unit");
		} else {
			println!("# would run: systemctl --user enable --now {unit_name}");
		}
		return Ok(());
	}

	let dir = unit_dir();
	std::fs::create_dir_all(&dir)
		.map_err(|e| ComposeError::Autostart(format!("cannot create {}: {e}", dir.display())))?;
	std::fs::write(&path, unit_text.as_bytes())
		.map_err(|e| ComposeError::Autostart(format!("cannot write {}: {e}", path.display())))?;
	eprintln!("podup: wrote {}", path.display());

	checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;
	if no_start {
		eprintln!("podup: installed {unit_name} (not enabled; --no-start)");
	} else {
		checked(
			sc.systemctl(&["enable", "--now", &unit_name]),
			&format!("enable --now {unit_name}"),
		)?;
		eprintln!("podup: enabled and started {unit_name}");
	}
	Ok(())
}

/// Write the auto-update oneshot service and its timer, then enable the timer
/// (not the service, the timer is what the user enables; the service runs
/// only when the timer fires). `interval` is the `OnCalendar=` word
/// (`hourly`/`daily`/`weekly`). The service is installed but not `enable
/// --now`'d: nothing would start it that way, since the timer is the entry
/// point and a disabled timer means nothing fires.
fn place_update_units<S: SystemCtl>(
	sc: &S,
	project: &str,
	main_opts: &super::service::ServiceUnitOpts,
	interval: &str,
	dry_run: bool,
	no_start: bool,
) -> crate::Result<()> {
	let service_text = render_update_service_unit(main_opts);
	let timer_text = render_update_timer_unit(project, interval);
	let service_name = update_service_file_name(project);
	let timer_name = update_timer_file_name(project);
	let dir = unit_dir();
	let service_path = dir.join(&service_name);
	let timer_path = dir.join(&timer_name);

	if dry_run {
		print!("{service_text}");
		print!("{timer_text}");
		println!("\n# would write {}", service_path.display());
		println!("# would write {}", timer_path.display());
		println!("# would run: systemctl --user daemon-reload");
		if no_start {
			println!("# (--no-start) would not enable or start {service_name} or {timer_name}");
		} else {
			println!("# would run: systemctl --user enable --now {timer_name}");
		}
		return Ok(());
	}

	std::fs::write(&service_path, service_text.as_bytes()).map_err(|e| {
		ComposeError::Autostart(format!("cannot write {}: {e}", service_path.display()))
	})?;
	eprintln!("podup: wrote {}", service_path.display());
	std::fs::write(&timer_path, timer_text.as_bytes()).map_err(|e| {
		ComposeError::Autostart(format!("cannot write {}: {e}", timer_path.display()))
	})?;
	eprintln!("podup: wrote {}", timer_path.display());

	checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;
	if !no_start {
		checked(
			sc.systemctl(&["enable", "--now", &timer_name]),
			&format!("enable --now {timer_name}"),
		)?;
		eprintln!("podup: enabled and started {timer_name}");
	} else {
		eprintln!("podup: installed {service_name} and {timer_name} (not enabled; --no-start)");
	}
	Ok(())
}

/// Whether systemd knows anything about `unit`: loaded, enabled, running, or
/// merely present as a fragment.
///
/// `systemctl is-active` exits **4** for a unit it has never heard of and
/// something else for every other state (0 active, 3 inactive/failed/activating).
/// That numeric 4 is the only reliable "there is nothing here" signal: the
/// message text is localised, and the *fragment file* is not a proxy for it:
/// measured, a unit whose file is deleted out of band stays loaded, enabled and
/// running, and `disable --now` still exits 0, removes its `.wants/` symlink and
/// stops it. Gating on the file would delete the only way out of that state.
///
/// A probe that cannot even be spawned returns `true`: the right response to
/// "I could not ask" is to attempt the disable anyway and let `checked` report
/// whatever happens, never to assume there is nothing to do.
fn unit_is_known<S: SystemCtl>(sc: &S, unit: &str) -> bool {
	sc.systemctl(&["is-active", "--quiet", unit])
		.map(|o| o.status.code() != Some(4))
		.unwrap_or(true)
}

/// Uninstall the service-mode autostart unit: disable + stop it, remove the unit
/// file, and reload the user manager.
///
/// Idempotent: uninstalling when nothing is installed is a quiet no-op, but a
/// `disable` that genuinely fails is reported rather than swallowed, so the
/// command cannot claim success while the service is still enabled and running.
///
/// When the auto-update timer pair is present, this removes both: the
/// `<unit>-update.service` oneshot and the `<unit>-update.timer` schedule that
/// fires it. Without that, the timer would keep firing `up -d` against a stack
/// whose main unit had been uninstalled, the exact inconsistency the issue
/// calls out.
pub fn uninstall<S: SystemCtl>(sc: &S, project: &str) -> crate::Result<()> {
	let unit_name = unit_file_name(project);
	let path = unit_path(project);

	// Disable whenever systemd knows the unit, whether or not its file is still
	// there. `disable --now` is idempotent across every state it can be in
	// (enabled, never enabled, running, stopped, fragment deleted out of band),
	// so the only case worth skipping is the one where systemd has never heard
	// of it, which is exactly what `unit_is_known` answers.
	if unit_is_known(sc, &unit_name) {
		checked(
			sc.systemctl(&["disable", "--now", &unit_name]),
			"disable --now",
		)?;
	}

	if path.exists() {
		std::fs::remove_file(&path).map_err(|e| {
			ComposeError::Autostart(format!("cannot remove {}: {e}", path.display()))
		})?;
		eprintln!("podup: removed {}", path.display());
	} else {
		eprintln!(
			"podup: no unit file at {} (already removed)",
			path.display()
		);
	}

	// The auto-update pair, when present. Run independently of the main unit's
	// outcome: the timer pair is its own install and uninstalls the same way.
	let update_service_name = update_service_file_name(project);
	let update_timer_name = update_timer_file_name(project);
	let update_service_path = unit_dir().join(&update_service_name);
	let update_timer_path = unit_dir().join(&update_timer_name);

	if unit_is_known(sc, &update_timer_name) {
		checked(
			sc.systemctl(&["disable", "--now", &update_timer_name]),
			&format!("disable --now {update_timer_name}"),
		)?;
	}
	if unit_is_known(sc, &update_service_name) {
		checked(
			sc.systemctl(&["disable", "--now", &update_service_name]),
			&format!("disable --now {update_service_name}"),
		)?;
	}
	for (_name, path) in [
		(update_service_name.as_str(), &update_service_path),
		(update_timer_name.as_str(), &update_timer_path),
	] {
		if path.exists() {
			std::fs::remove_file(path).map_err(|e| {
				ComposeError::Autostart(format!("cannot remove {}: {e}", path.display()))
			})?;
			eprintln!("podup: removed {}", path.display());
		}
	}

	checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;
	Ok(())
}
