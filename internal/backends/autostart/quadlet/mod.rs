//! `podup autostart --mode quadlet`: hand the whole stack to systemd as native
//! Podman Quadlet units under the rootless `~/.config/containers/systemd/`.
//!
//! Where service mode installs one unit that shells out to `podup up` at boot,
//! quadlet mode writes the same `.container`/`.build`/`.volume`/`.network` units
//! `generate quadlet` emits, so systemd owns boot, restart and dependency
//! ordering directly. The generated `.container` units already carry
//! `[Install] WantedBy=default.target`, so a `daemon-reload` wires them into boot
//! on its own, so this module writes them, reloads, and starts them now; it never
//! `enable`s a generated unit (systemd does not enable generator output).

use std::path::{Path, PathBuf};

use crate::compose::types::ComposeFile;
use crate::{quadlet, ComposeError};

mod drop_ins;
mod installed;
mod secret_paths;
mod stop;
pub(in super::super) use installed::installed_units;

use super::guards::emit_guards;
use super::paths::{config_home, unit_dir, unit_path};
use super::systemctl::{checked, SystemCtl};

/// `${XDG_CONFIG_HOME:-~/.config}/containers/systemd/`, where Quadlet reads a
/// user's units from. The same directory `generate quadlet` documents.
pub fn quadlet_dir() -> PathBuf {
	config_home().join("containers").join("systemd")
}

/// The `.service` names systemd derives from the generated `.container` units:
/// Quadlet turns `<stem>.container` into `<stem>.service`.
fn container_services(units: &[quadlet::QuadletUnit]) -> Vec<String> {
	units
		.iter()
		.filter_map(|u| u.filename.strip_suffix(".container"))
		.map(|stem| format!("{stem}.service"))
		.collect()
}

/// Build the lines `--dry-run` prints: every unit file (as `# <filename>` plus
/// its verbatim body), then a blank line, then the meta lines that describe
/// what the install would otherwise have invoked on the user manager. Split
/// out of [`install_quadlet`] so a regression in the wording (the exact
/// `# would run: systemctl --user restart ...` line in particular) can be
/// pinned in a unit test that does not have to capture stdout.
fn dry_run_plan(
	units: &[quadlet::QuadletUnit],
	drop_ins: &[drop_ins::DropIn],
	dir: &Path,
	services: &[String],
	build_services: &[String],
	no_start: bool,
) -> Vec<String> {
	let mut lines = Vec::new();
	for unit in units {
		lines.push(format!("# {}", unit.filename));
		for body in unit.contents.lines() {
			lines.push(body.to_string());
		}
	}
	for drop_in in drop_ins {
		lines.push(format!("# {}", drop_in.relative()));
		for body in drop_in.contents.lines() {
			lines.push(body.to_string());
		}
	}
	lines.push(String::new());
	lines.push(format!(
		"# would write {} unit(s) to {}",
		units.len(),
		dir.display()
	));
	if !drop_ins.is_empty() {
		lines.push(format!(
			"# would write {} drop-in(s) under {}",
			drop_ins.len(),
			unit_dir().display()
		));
	}
	lines.push("# would run: systemctl --user daemon-reload".to_string());
	// The build run is its own step: the container start depends on the
	// local image existing, and `--dry-run` must show both lines so an
	// operator can tell what install will actually do.
	if !build_services.is_empty() {
		lines.push(format!(
			"# would run: systemctl --user restart {}",
			build_services.join(" ")
		));
	}
	if no_start {
		lines.push("# (--no-start) would not start any container service".to_string());
	} else {
		for svc in services {
			lines.push(format!("# would run: systemctl --user start {svc}"));
		}
	}
	lines
}

/// Retire the units a previous install left behind that the new set does not
/// contain.
///
/// A service removed from the compose file keeps its unit, and with it its
/// container, which then starts again at every boot even though the project no
/// longer has that service.
///
/// Only the units this project owns are retired: the ownership marker decides
/// it, exactly as it does for the uninstall, so a sibling project's unit that
/// happens to share the prefix is left alone.
fn retire_obsolete_units<S: SystemCtl>(
	sc: &S,
	project: &str,
	previous: &[PathBuf],
	current: &[quadlet::QuadletUnit],
	protection: &[drop_ins::DropIn],
) -> crate::Result<()> {
	let mut retired = false;
	for path in previous {
		let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
			continue;
		};
		if current.iter().any(|unit| unit.filename == name) {
			continue;
		}
		// Stop the service while its unit still exists, so the container or the
		// pod it runs does not stay up with nothing left to manage it.
		if let Some(service) = drop_ins::service_of(name) {
			stop::service(sc, &service, protection)?;
		}
		std::fs::remove_file(path).map_err(|e| drop_ins::cannot("remove", path, e))?;
		retired = true;
		crate::ui::errln!("podup: removed the obsolete unit {}", path.display())?;
		if let Some(service) = drop_ins::service_of(name) {
			drop_ins::remove(project, &service)?;
		}
	}
	if retired {
		checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;
	}
	Ok(())
}

/// Install quadlet-mode autostart: render the stack's units in the prebuilt
/// shape, write them under `~/.config/containers/systemd/`, reload the user
/// manager, restart every `.build` service in one `systemctl --user restart
/// ...` call so the local images exist, then (unless `no_start`) start each
/// container service in one more call. Boot start comes from the units'
/// own `[Install] WantedBy=default.target`, so no `enable` is needed.
///
/// The prebuilt shape (see [`quadlet::generate_for_autostart`]) is what stops
/// the boot path from re-running `podman build`: in the standard
/// `podup generate quadlet` output a buildable service has `Image=<stem>.build`,
/// which makes Quadlet add `Requires=`/`After=` on the sibling build service,
/// and a `.build` service is a plain `Type=oneshot` without `RemainAfterExit=yes`,
/// so every container start (and every boot) re-runs `podman build` and a failed
/// build keeps the container down. Prebuilt mode gives it `Image=<tag>` and
/// `Pull=never` instead, so the dependency is gone and Quadlet never tries to
/// refresh the image; the build runs once here, and only on `autostart rebuild`.
pub fn install_quadlet<S: SystemCtl>(
	sc: &S,
	file: &ComposeFile,
	project: &str,
	base_dir: &Path,
	no_start: bool,
	dry_run: bool,
) -> crate::Result<()> {
	// Refuse to stack on top of a service-mode unit for the same project: both
	// would bring the same stack up at boot.
	let service = unit_path(project);
	if service.exists() {
		return Err(ComposeError::Autostart(format!(
			"service-mode autostart unit for '{project}' already exists at {}; \
			 remove it with `podup autostart uninstall` before installing quadlet mode \
			 (both would start the stack at boot).",
			service.display()
		)));
	}

	quadlet::validate_for_quadlet(file)?;
	let mut result = quadlet::generate_for_autostart(file, project, base_dir);
	if let Some(dup) = result.duplicate_filename() {
		return Err(ComposeError::Autostart(format!(
			"quadlet: two resources map to the same unit file or systemd service ({dup:?}); \
			 rename one so their names do not collide after sanitization."
		)));
	}
	for warning in &result.warnings {
		tracing::warn!("{warning}");
	}

	let dir = quadlet_dir();
	// Refuse an entry of the unit directory that is not a regular file before
	// anything is written, so the install stops and names the path instead of
	// writing its units and failing afterwards.
	let installed = installed_units(project)?;
	// The units of an install that 6.1.0 made mount every secret under its own
	// name, and the ones of today name the target after the compose secret.
	// Rewriting them would move the file a running service reads, so the
	// units this install is about to overwrite are read first and the path
	// each secret already has is kept.
	for kept in secret_paths::keep_installed_paths(&mut result.units, file, &installed) {
		crate::ui::errln!("{kept}")?;
	}
	let services = container_services(&result.units);
	// `podup autostart rebuild` is what restarts a `.build` unit to rebuild an
	// image, so the install path only has to build once. List the `.build`
	// services here (the same `result.units` we just validated against), in the
	// order the generator wrote them: the install path restarts them all on one
	// `systemctl --user restart ...` argv (so a re-install rebuilds a stale
	// image) and
	// `checked` propagates any non-zero exit, which is what stops the container
	// start from running after a failed build.
	let build_services: Vec<String> = result
		.units
		.iter()
		.filter(|u| u.filename.ends_with(".build"))
		// Quadlet turns `<stem>.build` into `<stem>-build.service` (note the
		// `-build`, not a literal strip), so what we restart is the same
		// service `podup autostart rebuild` does; matching its spelling
		// keeps the two paths aligned on a single `rebuild` call shape.
		.map(|u| format!("{}-build.service", u.filename.trim_end_matches(".build")))
		.collect();
	let mut drop_ins = drop_ins::plan(project, &result.units);
	emit_guards(sc);

	if dry_run {
		let plan = dry_run_plan(
			&result.units,
			&drop_ins,
			&dir,
			&services,
			&build_services,
			no_start,
		);
		for line in &plan {
			crate::ui::outln!("{line}")?;
		}
		return Ok(());
	}
	let mut old_drop_ins = drop_ins::existing(project, &installed)?;
	old_drop_ins.retain(|old| !drop_ins.iter().any(|new| new.same_service(old)));
	drop_ins.extend(old_drop_ins);

	let unwritable = |e: std::io::Error| {
		ComposeError::Autostart(format!(
			"cannot write quadlet units to {}: {e}",
			dir.display()
		))
	};
	// Both destinations, directories included, are checked before either is written, so a refusal
	// leaves nothing behind. The drop-ins go first: a unit that says `--replace=false` without its
	// drop-in still cleans up by name.
	crate::backends::unit_directory::check_private(&dir).map_err(unwritable)?;
	quadlet::check_unit_owners(&dir, &result.units).map_err(unwritable)?;
	drop_ins::check(project, &drop_ins)?;
	let written_drop_ins = drop_ins::write(&drop_ins)?;
	let written = quadlet::write_autostart_units(&dir, &result.units).map_err(unwritable)?;
	for path in written.iter().chain(&written_drop_ins) {
		crate::ui::errln!("podup: wrote {}", path.display())?;
	}
	// Load protection for existing services before any obsolete service is stopped.
	checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;

	// The units of the services the compose file no longer has are retired
	// here, so a re-install leaves no unit behind that would start a
	// container the project does not have any more.
	retire_obsolete_units(sc, project, &installed, &result.units, &drop_ins)?;

	// Build the images before the container services start. Building is not
	// "starting the stack": `--no-start` still does it, because without it the
	// first boot (which `--no-start` leaves to happen unattended) would fail
	// with "image not found". A stack with no `.build` units skips this call.
	//
	// `restart`, not `start`. Measured on Podman 6.2.0 (2026-11-22): a `.build`
	// service is `Type=oneshot` without `RemainAfterExit=yes`, so `start` on it
	// would also build again today. `restart` stays because it is what a
	// re-install needs whatever the unit type is: on an inactive unit it
	// starts it, on an active one (a `RemainAfterExit=yes` oneshot, which an
	// older or later Quadlet may write) it re-runs the build instead of
	// short-circuiting, and it is what `podup autostart rebuild` does.
	if !build_services.is_empty() {
		let mut build_args: Vec<&str> = Vec::with_capacity(build_services.len() + 1);
		build_args.push("restart");
		for svc in &build_services {
			build_args.push(svc.as_str());
		}
		checked(
			sc.systemctl(&build_args),
			&format!("restart {}", build_services.join(" ")),
		)?;
		crate::ui::errln!(
			"podup: built {} image(s) for '{project}'",
			build_services.len()
		)?;
	}
	if no_start {
		crate::ui::errln!(
			"podup: installed {} quadlet unit(s) for '{project}' (not started; --no-start)",
			written.len()
		)?;
	} else {
		// One `systemctl --user start svc1 svc2 ...` call instead of one fork
		// per service: systemd serializes the starts whether they were the
		// operands of a single call or separate ones, so the only difference
		// at the binary level was the per-service process fork (#1747).
		let svc_args: Vec<&str> = services.iter().map(String::as_str).collect();
		let mut args: Vec<&str> = Vec::with_capacity(svc_args.len() + 1);
		args.push("start");
		args.extend(svc_args);
		checked(
			sc.systemctl(&args),
			&format!("start {}", services.join(" ")),
		)?;
		crate::ui::errln!(
			"podup: started {} container service(s) for '{project}'",
			services.len()
		)?;
	}
	Ok(())
}

/// Uninstall quadlet-mode autostart: stop this project's container services, remove
/// its `<project>-*` unit files, and reload the user manager. Idempotent: a
/// service that was never started, or a file already gone, is not an error.
///
/// A failed stop or cleanup retains the units and their protection for retry.
pub fn uninstall_quadlet<S: SystemCtl>(sc: &S, project: &str) -> crate::Result<()> {
	let units = installed_units(project)?;
	let protection = drop_ins::existing(project, &units)?;
	drop_ins::check(project, &protection)?;
	drop_ins::write(&protection)?;
	if !protection.is_empty() {
		checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;
	}
	let mut first_err: Option<ComposeError> = None;
	// Stop the container services first, then the pod's, while their units still
	// exist: the units are sorted, and `<project>-` sorts before `<project>.`.
	for path in &units {
		if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
			if let Some(service) = drop_ins::service_of(name) {
				if let Err(e) = stop::service(sc, &service, &protection) {
					tracing::warn!("{e}");
					first_err.get_or_insert(e);
				}
			}
		}
	}
	if let Some(e) = first_err {
		return Err(e);
	}
	let mut removed = 0usize;
	for path in &units {
		std::fs::remove_file(path).map_err(|e| drop_ins::cannot("remove", path, e))?;
		crate::ui::errln!("podup: removed {}", path.display())?;
		removed += 1;
		if let Some(service) = path
			.file_name()
			.and_then(|n| n.to_str())
			.and_then(drop_ins::service_of)
		{
			drop_ins::remove(project, &service)?;
		}
	}
	if removed == 0 {
		crate::ui::errln!("podup: no quadlet autostart units for '{project}' (already removed)")?;
	}
	checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;
	Ok(())
}

/// Rebuild one or all built images of a quadlet-mode install. A `.build` unit is
/// `Type=oneshot`, so its image only rebuilds when the build service is restarted;
/// the container is then restarted to pick up the new image. With `service` given,
/// only that service rebuilds; otherwise every service that has a `.build` unit.
///
/// The install path (`install_quadlet`) writes the prebuilt shape, so the
/// restarted container carries `Pull=never` and reads the freshly built local
/// image at restart, with no registry hop. This is the path that makes the build
/// service `Type=oneshot` (not `RemainAfterExit=yes`) usable again: the install
/// builds once, `rebuild` rebuilds on demand, and boot (and any subsequent
/// container restart) just runs the cached result.
pub fn rebuild_quadlet<S: SystemCtl>(
	sc: &S,
	project: &str,
	service: Option<&str>,
) -> crate::Result<()> {
	let prefix = format!("{project}-");
	let builds: Vec<String> = installed_units(project)?
		.iter()
		.filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
		.filter_map(|n| n.strip_suffix(".build").map(String::from))
		.collect();
	if builds.is_empty() {
		return Err(ComposeError::Autostart(format!(
			"no quadlet build units for '{project}'; nothing to rebuild. Only a service \
			 with a compose `build:` produces a `.build` unit, and quadlet-mode autostart \
			 must be installed first (`podup autostart install --mode quadlet`)."
		)));
	}
	let targets: Vec<String> = match service {
		Some(svc) => {
			let stem = format!("{prefix}{svc}");
			if !builds.contains(&stem) {
				let names: Vec<&str> = builds
					.iter()
					.map(|b| b.strip_prefix(&prefix).unwrap_or(b))
					.collect();
				return Err(ComposeError::Autostart(format!(
					"service '{svc}' has no build unit under '{project}'; built services are: {}",
					names.join(", ")
				)));
			}
			vec![stem]
		}
		None => builds,
	};
	for stem in &targets {
		checked(
			sc.systemctl(&["restart", &format!("{stem}-build.service")]),
			&format!("restart {stem}-build.service"),
		)?;
		checked(
			sc.systemctl(&["restart", &format!("{stem}.service")]),
			&format!("restart {stem}.service"),
		)?;
		crate::ui::errln!("podup: rebuilt {stem}")?;
	}
	Ok(())
}

// Unix-gated: the fake `SystemCtl` builds `Output`s via `os::unix` and the paths
// asserted are POSIX. Autostart is a `systemctl --user` feature, so this matches
// the service-mode tests, which gate the same way.
#[cfg(all(test, unix))]
mod tests;
