//! `podup autostart --mode quadlet`: hand the whole stack to systemd as native
//! Podman Quadlet units under the rootless `~/.config/containers/systemd/`.
//!
//! Where service mode installs one unit that shells out to `podup up` at boot,
//! quadlet mode writes the same `.container`/`.build`/`.volume`/`.network` units
//! `generate quadlet` emits, so systemd owns boot, restart and dependency
//! ordering directly. The generated `.container` units already carry
//! `[Install] WantedBy=default.target`, so a `daemon-reload` wires them into boot
//! on its own — this module writes them, reloads, and starts them now; it never
//! `enable`s a generated unit (systemd does not enable generator output).

use std::path::{Path, PathBuf};

use crate::compose::types::ComposeFile;
use crate::{quadlet, ComposeError};

use super::{checked, config_home, emit_guards, unit_path, SystemCtl};

/// `${XDG_CONFIG_HOME:-~/.config}/containers/systemd/` — where Quadlet reads a
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

/// This project's installed quadlet unit files (`<project>-*`), sorted. Drives
/// uninstall (remove by prefix) and rebuild (find `.build` units).
fn installed_units(project: &str) -> Vec<PathBuf> {
	let dir = quadlet_dir();
	let prefix = format!("{project}-");
	let mut found = Vec::new();
	if let Ok(entries) = std::fs::read_dir(&dir) {
		for entry in entries.flatten() {
			if entry.file_name().to_string_lossy().starts_with(&prefix) {
				found.push(entry.path());
			}
		}
	}
	found.sort();
	found
}

/// Install quadlet-mode autostart: render the stack's units, write them under
/// `~/.config/containers/systemd/`, reload the user manager, and (unless
/// `no_start`) start each container service now. Boot start comes from the units'
/// own `[Install] WantedBy=default.target`, so no `enable` is needed.
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

	let result = quadlet::generate_at(file, project, base_dir);
	if let Some(dup) = result.duplicate_filename() {
		return Err(ComposeError::Autostart(format!(
			"quadlet: two resources map to the same unit file {dup:?}; \
			 rename one so their names do not collide after sanitization."
		)));
	}
	for warning in &result.warnings {
		eprintln!("podup: warning: {warning}");
	}

	let dir = quadlet_dir();
	let services = container_services(&result.units);
	emit_guards(sc);

	if dry_run {
		for unit in &result.units {
			println!("# {}", unit.filename);
			print!("{}", unit.contents);
		}
		println!(
			"\n# would write {} unit(s) to {}",
			result.units.len(),
			dir.display()
		);
		println!("# would run: systemctl --user daemon-reload");
		if no_start {
			println!("# (--no-start) would not start any container service");
		} else {
			for svc in &services {
				println!("# would run: systemctl --user start {svc}");
			}
		}
		return Ok(());
	}

	let written = quadlet::write_units(&dir, &result.units).map_err(|e| {
		ComposeError::Autostart(format!(
			"cannot write quadlet units to {}: {e}",
			dir.display()
		))
	})?;
	for path in &written {
		eprintln!("podup: wrote {}", path.display());
	}

	checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;
	if no_start {
		eprintln!(
			"podup: installed {} quadlet unit(s) for '{project}' (not started; --no-start)",
			written.len()
		);
	} else {
		for svc in &services {
			checked(sc.systemctl(&["start", svc]), &format!("start {svc}"))?;
		}
		eprintln!(
			"podup: started {} container service(s) for '{project}'",
			services.len()
		);
	}
	Ok(())
}

/// Uninstall quadlet-mode autostart: stop this project's container services, remove
/// its `<project>-*` unit files, and reload the user manager. Idempotent — a
/// service that was never loaded, or a file already gone, is not an error.
pub fn uninstall_quadlet<S: SystemCtl>(sc: &S, project: &str) -> crate::Result<()> {
	let units = installed_units(project);
	// Stop the container services first (best-effort) while their units still exist.
	for path in &units {
		if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
			if let Some(stem) = name.strip_suffix(".container") {
				let _ = sc.systemctl(&["stop", &format!("{stem}.service")]);
			}
		}
	}
	let mut removed = 0usize;
	for path in &units {
		std::fs::remove_file(path).map_err(|e| {
			ComposeError::Autostart(format!("cannot remove {}: {e}", path.display()))
		})?;
		eprintln!("podup: removed {}", path.display());
		removed += 1;
	}
	if removed == 0 {
		eprintln!("podup: no quadlet autostart units for '{project}' (already removed)");
	}
	checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;
	Ok(())
}

/// Rebuild one or all built images of a quadlet-mode install. A `.build` unit is
/// `Type=oneshot`, so its image only rebuilds when the build service is restarted;
/// the container is then restarted to pick up the new image. With `service` given,
/// only that service rebuilds; otherwise every service that has a `.build` unit.
pub fn rebuild_quadlet<S: SystemCtl>(
	sc: &S,
	project: &str,
	service: Option<&str>,
) -> crate::Result<()> {
	let prefix = format!("{project}-");
	let builds: Vec<String> = installed_units(project)
		.iter()
		.filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
		.filter_map(|n| n.strip_suffix(".build").map(String::from))
		.collect();
	if builds.is_empty() {
		return Err(ComposeError::Autostart(format!(
			"no quadlet build units for '{project}' — nothing to rebuild. Only a service \
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
		eprintln!("podup: rebuilt {stem}");
	}
	Ok(())
}

// Unix-gated: the fake `SystemCtl` builds `Output`s via `os::unix` and the paths
// asserted are POSIX. Autostart is a `systemctl --user` feature, so this matches
// the service-mode tests, which gate the same way.
#[cfg(all(test, unix))]
mod tests {
	use super::{container_services, install_quadlet, rebuild_quadlet, uninstall_quadlet};
	use crate::autostart::SystemCtl;
	use crate::{parse_str, quadlet};
	use std::cell::RefCell;
	use std::os::unix::process::ExitStatusExt;
	use std::path::Path;
	use std::process::{ExitStatus, Output};

	/// Records every `systemctl` arg vector; `loginctl` always reports linger on.
	struct FakeCtl {
		calls: RefCell<Vec<Vec<String>>>,
	}
	impl FakeCtl {
		fn new() -> Self {
			FakeCtl {
				calls: RefCell::new(Vec::new()),
			}
		}
		fn log(&self) -> Vec<Vec<String>> {
			self.calls.borrow().clone()
		}
	}
	impl SystemCtl for FakeCtl {
		fn systemctl(&self, args: &[&str]) -> std::io::Result<Output> {
			self.calls
				.borrow_mut()
				.push(args.iter().map(|s| s.to_string()).collect());
			Ok(Output {
				status: ExitStatus::from_raw(0),
				stdout: Vec::new(),
				stderr: Vec::new(),
			})
		}
		fn loginctl(&self, _args: &[&str]) -> std::io::Result<Output> {
			Ok(Output {
				status: ExitStatus::from_raw(0),
				stdout: b"yes".to_vec(),
				stderr: Vec::new(),
			})
		}
	}

	/// Run `f` with a fresh temp `XDG_CONFIG_HOME`/`XDG_RUNTIME_DIR`/`USER`, so
	/// `quadlet_dir` and the guards resolve under the temp dir.
	fn with_env<R>(f: impl FnOnce(&Path) -> R) -> R {
		let tmp = tempfile::tempdir().unwrap();
		let root = tmp.path().to_path_buf();
		temp_env::with_vars(
			[
				("XDG_CONFIG_HOME", Some(root.as_os_str())),
				("XDG_RUNTIME_DIR", Some(root.as_os_str())),
				("USER", Some(std::ffi::OsStr::new("tester"))),
			],
			|| f(&root),
		)
	}

	const IMG: &str = "services:\n  web:\n    image: nginx\n";
	const BUILD: &str = "services:\n  web:\n    build: .\n";
	const BASE: &str = "/srv/app";

	#[test]
	fn container_services_names_only_containers() {
		let file = parse_str(IMG).unwrap();
		let units = quadlet::generate_at(&file, "proj", Path::new(BASE)).units;
		// The default network unit is present too, but only `.container`s become services.
		assert_eq!(
			container_services(&units),
			vec!["proj-web.service".to_string()]
		);
	}

	#[test]
	fn install_writes_units_reloads_then_starts() {
		with_env(|root| {
			let sc = FakeCtl::new();
			install_quadlet(
				&sc,
				&parse_str(IMG).unwrap(),
				"proj",
				Path::new(BASE),
				false,
				false,
			)
			.unwrap();
			assert!(root.join("containers/systemd/proj-web.container").is_file());
			let calls = sc.log();
			assert_eq!(calls[0], vec!["daemon-reload"]);
			assert_eq!(calls[1], vec!["start", "proj-web.service"]);
		});
	}

	#[test]
	fn no_start_reloads_but_starts_nothing() {
		with_env(|root| {
			let sc = FakeCtl::new();
			install_quadlet(
				&sc,
				&parse_str(IMG).unwrap(),
				"proj",
				Path::new(BASE),
				true,
				false,
			)
			.unwrap();
			assert!(root.join("containers/systemd/proj-web.container").is_file());
			assert_eq!(sc.log(), vec![vec!["daemon-reload".to_string()]]);
		});
	}

	#[test]
	fn dry_run_writes_nothing_and_runs_no_systemctl() {
		with_env(|root| {
			let sc = FakeCtl::new();
			install_quadlet(
				&sc,
				&parse_str(IMG).unwrap(),
				"proj",
				Path::new(BASE),
				false,
				true,
			)
			.unwrap();
			assert!(!root.join("containers/systemd/proj-web.container").exists());
			assert!(sc.log().is_empty());
		});
	}

	#[test]
	fn install_refuses_when_service_mode_is_present() {
		with_env(|root| {
			let sd = root.join("systemd/user");
			std::fs::create_dir_all(&sd).unwrap();
			std::fs::write(sd.join("podup-proj.service"), "x").unwrap();
			let err = install_quadlet(
				&FakeCtl::new(),
				&parse_str(IMG).unwrap(),
				"proj",
				Path::new(BASE),
				false,
				false,
			)
			.unwrap_err();
			assert!(format!("{err}").contains("service-mode autostart unit"));
		});
	}

	#[test]
	fn uninstall_stops_removes_and_reloads() {
		with_env(|root| {
			install_quadlet(
				&FakeCtl::new(),
				&parse_str(IMG).unwrap(),
				"proj",
				Path::new(BASE),
				true,
				false,
			)
			.unwrap();
			let sc = FakeCtl::new();
			uninstall_quadlet(&sc, "proj").unwrap();
			assert!(!root.join("containers/systemd/proj-web.container").exists());
			let calls = sc.log();
			assert!(calls.contains(&vec!["stop".to_string(), "proj-web.service".to_string()]));
			assert_eq!(calls.last().unwrap(), &vec!["daemon-reload".to_string()]);
		});
	}

	#[test]
	fn rebuild_restarts_build_then_container() {
		with_env(|_root| {
			install_quadlet(
				&FakeCtl::new(),
				&parse_str(BUILD).unwrap(),
				"proj",
				Path::new(BASE),
				true,
				false,
			)
			.unwrap();
			let sc = FakeCtl::new();
			rebuild_quadlet(&sc, "proj", Some("web")).unwrap();
			assert_eq!(
				sc.log(),
				vec![
					vec!["restart".to_string(), "proj-web-build.service".to_string()],
					vec!["restart".to_string(), "proj-web.service".to_string()],
				]
			);
		});
	}

	#[test]
	fn rebuild_unknown_service_errors_and_lists_valid_ones() {
		with_env(|_root| {
			install_quadlet(
				&FakeCtl::new(),
				&parse_str(BUILD).unwrap(),
				"proj",
				Path::new(BASE),
				true,
				false,
			)
			.unwrap();
			let err = rebuild_quadlet(&FakeCtl::new(), "proj", Some("nope")).unwrap_err();
			let msg = format!("{err}");
			assert!(msg.contains("has no build unit"), "{msg}");
			assert!(msg.contains("web"), "{msg}");
		});
	}
}
