//! `podup autostart` service mode: a single rootless `systemctl --user` unit that
//! brings a compose stack up at boot.
//!
//! Everything here is user-scope only — the unit lives under
//! `${XDG_CONFIG_HOME:-~/.config}/systemd/user/` and every action goes through
//! `systemctl --user` / `loginctl`. No root, no `sudo`, nothing under `/etc` or
//! the system systemd. External-command calls go through the `SystemCtl` seam so
//! the install/uninstall/status logic is unit-testable without a live systemd.

mod service;

pub use service::{render_service_unit, ServiceUnitOpts};

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::ComposeError;

/// Seam over the external `systemctl --user` / `loginctl` commands. The real impl
/// shells out; tests substitute a fake that records the argument vectors and
/// returns canned output, so install/uninstall/status are exercised without
/// touching the host's systemd.
pub trait SystemCtl {
	/// Run `systemctl --user <args>`.
	fn systemctl(&self, args: &[&str]) -> io::Result<Output>;
	/// Run `loginctl <args>`.
	fn loginctl(&self, args: &[&str]) -> io::Result<Output>;
}

/// The production [`SystemCtl`]: invokes the real `systemctl --user` and
/// `loginctl` binaries.
pub struct RealSystemCtl;

impl SystemCtl for RealSystemCtl {
	fn systemctl(&self, args: &[&str]) -> io::Result<Output> {
		Command::new("systemctl").arg("--user").args(args).output()
	}

	fn loginctl(&self, args: &[&str]) -> io::Result<Output> {
		Command::new("loginctl").args(args).output()
	}
}

/// Options for [`install`].
pub struct InstallOptions {
	/// The unit to render and install.
	pub unit: ServiceUnitOpts,
	/// Install the unit but do not `enable --now` it (no immediate start).
	pub no_start: bool,
	/// Print the unit and the actions that would run, but change nothing.
	pub dry_run: bool,
}

/// `${XDG_CONFIG_HOME:-~/.config}`. Falls back to `$HOME/.config`, then `.config`
/// in the working directory if even `HOME` is unset (so a path is always formed).
fn config_home() -> PathBuf {
	if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()) {
		return PathBuf::from(x);
	}
	match std::env::var_os("HOME").filter(|s| !s.is_empty()) {
		Some(home) => PathBuf::from(home).join(".config"),
		None => PathBuf::from(".config"),
	}
}

/// Directory that holds `systemctl --user` unit files.
fn unit_dir() -> PathBuf {
	config_home().join("systemd").join("user")
}

/// The unit's file name: `podup-<project>.service`. The project name is validated
/// as a safe path component before reaching here, so it cannot escape `unit_dir`.
fn unit_file_name(project: &str) -> String {
	format!("podup-{project}.service")
}

/// Full path to the unit file for a project.
fn unit_path(project: &str) -> PathBuf {
	unit_dir().join(unit_file_name(project))
}

/// The current login user, for `loginctl` and linger queries. Read from the
/// environment to avoid an `unsafe` `getuid` FFI call.
fn current_user() -> Option<String> {
	std::env::var("USER")
		.ok()
		.or_else(|| std::env::var("LOGNAME").ok())
		.filter(|s| !s.is_empty())
}

/// Quadlet autostart units for this project, if any exist on disk. Service mode
/// and Quadlet mode would both try to start the same stack at boot, so an
/// existing Quadlet install is a conflict to surface, not to silently overwrite.
/// Looks for `<project>-*.container` under
/// `${XDG_CONFIG_HOME:-~/.config}/containers/systemd/`.
fn quadlet_units_present(project: &str) -> Vec<PathBuf> {
	let dir = config_home().join("containers").join("systemd");
	let prefix = format!("{project}-");
	let mut found = Vec::new();
	if let Ok(entries) = std::fs::read_dir(&dir) {
		for entry in entries.flatten() {
			let name = entry.file_name();
			let name = name.to_string_lossy();
			if name.starts_with(&prefix) && name.ends_with(".container") {
				found.push(entry.path());
			}
		}
	}
	found.sort();
	found
}

/// Whether linger is enabled for `user` (so the user manager — and the stack —
/// survives logout and starts at boot). Parses `loginctl show-user <user>
/// --value --property=Linger`, treating any error/unexpected output as "off".
fn linger_enabled<S: SystemCtl>(sc: &S, user: &str) -> bool {
	match sc.loginctl(&["show-user", user, "--value", "--property=Linger"]) {
		Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
			.trim()
			.eq_ignore_ascii_case("yes"),
		_ => false,
	}
}

/// Advisory when linger is off: without it the user manager is not started at
/// boot, so the unit will not bring the stack up until first login. Returns the
/// message to print, or `None` when linger is already enabled.
fn linger_warning<S: SystemCtl>(sc: &S) -> Option<String> {
	let user = current_user()?;
	if linger_enabled(sc, &user) {
		return None;
	}
	Some(format!(
		"linger is not enabled for {user}; the stack will not start at boot until you run:\n    \
		 loginctl enable-linger {user}"
	))
}

/// Advisory when `XDG_RUNTIME_DIR` is unset: `systemctl --user` needs a live user
/// session bus, which is otherwise missing (so the calls below will likely fail).
/// Returns the message to print, or `None` when the variable is set.
fn runtime_dir_warning() -> Option<String> {
	let present = std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|s| !s.is_empty());
	if present {
		return None;
	}
	Some(
		"XDG_RUNTIME_DIR is not set; `systemctl --user` needs an active user session. \
		 Open one (e.g. `machinectl shell <user>@`) or export XDG_RUNTIME_DIR before retrying."
			.to_string(),
	)
}

/// Print the linger and runtime-dir advisories to stderr (warn, never fail).
fn emit_guards<S: SystemCtl>(sc: &S) {
	for warning in [linger_warning(sc), runtime_dir_warning()]
		.into_iter()
		.flatten()
	{
		eprintln!("podup: warning: {warning}");
	}
}

/// Turn a `systemctl` invocation result into a `Result`, mapping a launch failure
/// or a non-zero exit into a clear autostart error naming the action.
fn checked(res: io::Result<Output>, what: &str) -> crate::Result<()> {
	let out = res.map_err(|e| {
		ComposeError::Autostart(format!("failed to run `systemctl --user {what}`: {e}"))
	})?;
	if out.status.success() {
		return Ok(());
	}
	let stderr = String::from_utf8_lossy(&out.stderr);
	Err(ComposeError::Autostart(format!(
		"`systemctl --user {what}` failed: {}",
		stderr.trim()
	)))
}

/// Install (and, unless `no_start`, enable + start) the service-mode autostart
/// unit. Writes only under `${XDG_CONFIG_HOME:-~/.config}/systemd/user/`.
pub fn install<S: SystemCtl>(sc: &S, opts: &InstallOptions) -> crate::Result<()> {
	let project = &opts.unit.project;
	let path = unit_path(project);

	// Refuse to stack service mode on top of an existing Quadlet autostart install
	// for the same project — both would start the stack at boot.
	let quadlet = quadlet_units_present(project);
	if !quadlet.is_empty() {
		let names: Vec<String> = quadlet.iter().map(|p| p.display().to_string()).collect();
		return Err(ComposeError::Autostart(format!(
			"quadlet autostart units for project '{project}' already exist:\n    {}\n\
			 remove them before installing service mode (quadlet autostart is tracked by #993).",
			names.join("\n    ")
		)));
	}

	let unit_text = render_service_unit(&opts.unit);
	let unit_name = unit_file_name(project);

	// Surface the linger / session guards before acting (or previewing).
	emit_guards(sc);

	if opts.dry_run {
		print!("{unit_text}");
		println!("\n# would write {}", path.display());
		println!("# would run: systemctl --user daemon-reload");
		if opts.no_start {
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
	if opts.no_start {
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

/// Uninstall the service-mode autostart unit: disable + stop it, remove the unit
/// file, and reload the user manager. A "unit not loaded" disable failure is
/// ignored so uninstall is idempotent.
pub fn uninstall<S: SystemCtl>(sc: &S, project: &str) -> crate::Result<()> {
	let unit_name = unit_file_name(project);
	// Best-effort: ignore failures (e.g. the unit was never loaded).
	let _ = sc.systemctl(&["disable", "--now", &unit_name]);

	let path = unit_path(project);
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

	checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;
	Ok(())
}

/// A snapshot of the autostart unit's state, gathered for `status`.
pub struct StatusReport {
	/// Absolute path to where the unit file would live.
	pub unit_path: PathBuf,
	/// Whether the unit file exists on disk.
	pub unit_exists: bool,
	/// The unit file's permission bits (Unix only), when it exists.
	pub unit_mode: Option<u32>,
	/// `systemctl --user is-active` output (e.g. `active`, `inactive`, `failed`).
	pub is_active: String,
	/// `systemctl --user is-enabled` output (e.g. `enabled`, `disabled`).
	pub is_enabled: String,
	/// Whether linger is enabled for the current user.
	pub linger: bool,
	/// Whether `XDG_RUNTIME_DIR` is set (a user session is present).
	pub runtime_dir: bool,
}

/// Read the unit file's permission bits, on Unix. Other platforms have no POSIX
/// mode, so this is always `None` there.
#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
	use std::os::unix::fs::PermissionsExt;
	std::fs::metadata(path).ok().map(|m| m.permissions().mode())
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Option<u32> {
	None
}

/// Run a `systemctl --user` query that reports state through its stdout (e.g.
/// `is-active`, `is-enabled`); these exit non-zero for the negative answer, so
/// the trimmed stdout is the report regardless of exit status.
fn query<S: SystemCtl>(sc: &S, arg: &str, unit_name: &str) -> String {
	match sc.systemctl(&[arg, unit_name]) {
		Ok(out) => {
			let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
			if s.is_empty() {
				"unknown".to_string()
			} else {
				s
			}
		}
		Err(e) => format!("unknown ({e})"),
	}
}

/// Gather the autostart status for a project, going through the [`SystemCtl`] seam
/// so it is testable without a live systemd.
pub fn collect_status<S: SystemCtl>(sc: &S, project: &str) -> StatusReport {
	let unit_name = unit_file_name(project);
	let path = unit_path(project);
	let unit_exists = path.exists();
	StatusReport {
		unit_mode: if unit_exists { file_mode(&path) } else { None },
		unit_exists,
		unit_path: path,
		is_active: query(sc, "is-active", &unit_name),
		is_enabled: query(sc, "is-enabled", &unit_name),
		linger: current_user().is_some_and(|u| linger_enabled(sc, &u)),
		runtime_dir: std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|s| !s.is_empty()),
	}
}

/// Print the autostart status for a project.
pub fn status<S: SystemCtl>(sc: &S, project: &str) -> crate::Result<()> {
	let r = collect_status(sc, project);
	println!("unit:       {}", r.unit_path.display());
	println!("installed:  {}", if r.unit_exists { "yes" } else { "no" });
	if let Some(mode) = r.unit_mode {
		println!("mode:       {:04o}", mode & 0o7777);
	}
	println!("active:     {}", r.is_active);
	println!("enabled:    {}", r.is_enabled);
	println!(
		"linger:     {}",
		if r.linger { "enabled" } else { "disabled" }
	);
	println!(
		"session:    {}",
		if r.runtime_dir {
			"XDG_RUNTIME_DIR set"
		} else {
			"XDG_RUNTIME_DIR unset (systemctl --user needs a user session)"
		}
	);
	Ok(())
}

#[cfg(all(test, unix))]
mod tests {
	use super::*;
	use std::cell::RefCell;
	use std::os::unix::process::ExitStatusExt;
	use std::process::ExitStatus;

	/// Recording fake: captures every `systemctl`/`loginctl` arg vector and returns
	/// canned output keyed off the first argument.
	struct FakeCtl {
		systemctl_calls: RefCell<Vec<Vec<String>>>,
		loginctl_calls: RefCell<Vec<Vec<String>>>,
		linger: String,
		is_active: String,
		is_enabled: String,
		fail: bool,
	}

	impl FakeCtl {
		fn new() -> Self {
			FakeCtl {
				systemctl_calls: RefCell::new(Vec::new()),
				loginctl_calls: RefCell::new(Vec::new()),
				linger: "yes".to_string(),
				is_active: "active".to_string(),
				is_enabled: "enabled".to_string(),
				fail: false,
			}
		}

		fn systemctl_log(&self) -> Vec<Vec<String>> {
			self.systemctl_calls.borrow().clone()
		}
	}

	fn out(code: i32, stdout: &str) -> Output {
		Output {
			status: ExitStatus::from_raw(code),
			stdout: stdout.as_bytes().to_vec(),
			stderr: Vec::new(),
		}
	}

	impl SystemCtl for FakeCtl {
		fn systemctl(&self, args: &[&str]) -> io::Result<Output> {
			self.systemctl_calls
				.borrow_mut()
				.push(args.iter().map(|s| s.to_string()).collect());
			let code = if self.fail { 256 } else { 0 };
			let stdout = match args.first().copied() {
				Some("is-active") => self.is_active.as_str(),
				Some("is-enabled") => self.is_enabled.as_str(),
				_ => "",
			};
			// is-active/is-enabled report through stdout; their exit status is not
			// consulted, so leave it successful for those queries.
			let code = if matches!(args.first().copied(), Some("is-active" | "is-enabled")) {
				0
			} else {
				code
			};
			Ok(out(code, stdout))
		}

		fn loginctl(&self, args: &[&str]) -> io::Result<Output> {
			self.loginctl_calls
				.borrow_mut()
				.push(args.iter().map(|s| s.to_string()).collect());
			Ok(out(0, &self.linger))
		}
	}

	fn opts(dir: &Path, project: &str, dry_run: bool, no_start: bool) -> InstallOptions {
		InstallOptions {
			unit: ServiceUnitOpts {
				exe: PathBuf::from("/usr/local/bin/podup"),
				compose_files: vec![dir.join("docker-compose.yml")],
				project: project.to_string(),
				working_dir: dir.to_path_buf(),
				profiles: Vec::new(),
				env_files: Vec::new(),
			},
			no_start,
			dry_run,
		}
	}

	/// Run `f` with a fresh temp `XDG_CONFIG_HOME`, `USER`, and `XDG_RUNTIME_DIR`
	/// set, so the install/status paths resolve under the temp dir.
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

	#[test]
	fn install_writes_unit_and_enables() {
		with_env(|root| {
			let sc = FakeCtl::new();
			install(&sc, &opts(root, "app", false, false)).unwrap();
			let path = root.join("systemd/user/podup-app.service");
			assert!(path.is_file(), "unit file written");
			let body = std::fs::read_to_string(&path).unwrap();
			assert!(body.contains("Description=podup app"));
			let calls = sc.systemctl_log();
			assert_eq!(calls[0], vec!["daemon-reload"]);
			assert_eq!(calls[1], vec!["enable", "--now", "podup-app.service"]);
		});
	}

	#[test]
	fn install_no_start_skips_enable() {
		with_env(|root| {
			let sc = FakeCtl::new();
			install(&sc, &opts(root, "app", false, true)).unwrap();
			let calls = sc.systemctl_log();
			assert_eq!(calls, vec![vec!["daemon-reload"]]);
		});
	}

	#[test]
	fn dry_run_writes_nothing_and_runs_no_systemctl() {
		with_env(|root| {
			let sc = FakeCtl::new();
			install(&sc, &opts(root, "app", true, false)).unwrap();
			assert!(!root.join("systemd/user/podup-app.service").exists());
			assert!(sc.systemctl_log().is_empty());
		});
	}

	#[test]
	fn uninstall_disables_removes_and_reloads() {
		with_env(|root| {
			let sc = FakeCtl::new();
			install(&sc, &opts(root, "app", false, true)).unwrap();
			let path = root.join("systemd/user/podup-app.service");
			assert!(path.exists());

			let sc2 = FakeCtl::new();
			uninstall(&sc2, "app").unwrap();
			assert!(!path.exists(), "unit file removed");
			let calls = sc2.systemctl_log();
			assert_eq!(calls[0], vec!["disable", "--now", "podup-app.service"]);
			assert_eq!(calls[1], vec!["daemon-reload"]);
		});
	}

	#[test]
	fn install_refuses_on_quadlet_conflict() {
		with_env(|root| {
			let qdir = root.join("containers/systemd");
			std::fs::create_dir_all(&qdir).unwrap();
			std::fs::write(qdir.join("app-web.container"), b"[Container]\n").unwrap();
			let sc = FakeCtl::new();
			let err = install(&sc, &opts(root, "app", false, false)).unwrap_err();
			assert!(matches!(err, ComposeError::Autostart(_)));
			assert!(err.to_string().contains("quadlet"));
			// Nothing was installed.
			assert!(!root.join("systemd/user/podup-app.service").exists());
			assert!(sc.systemctl_log().is_empty());
		});
	}

	#[test]
	fn linger_off_produces_warning() {
		let mut sc = FakeCtl::new();
		sc.linger = "no".to_string();
		temp_env::with_var("USER", Some("tester"), || {
			assert!(linger_warning(&sc).is_some());
		});
	}

	#[test]
	fn linger_on_produces_no_warning() {
		let sc = FakeCtl::new(); // linger = "yes"
		temp_env::with_var("USER", Some("tester"), || {
			assert!(linger_warning(&sc).is_none());
		});
	}

	#[test]
	fn missing_runtime_dir_produces_warning() {
		temp_env::with_var_unset("XDG_RUNTIME_DIR", || {
			assert!(runtime_dir_warning().is_some());
		});
		temp_env::with_var("XDG_RUNTIME_DIR", Some("/run/user/1000"), || {
			assert!(runtime_dir_warning().is_none());
		});
	}

	#[test]
	fn collect_status_reports_installed_and_state() {
		with_env(|root| {
			let sc = FakeCtl::new();
			install(&sc, &opts(root, "app", false, true)).unwrap();
			let r = collect_status(&sc, "app");
			assert!(r.unit_exists);
			assert!(r.unit_mode.is_some());
			assert_eq!(r.is_active, "active");
			assert_eq!(r.is_enabled, "enabled");
			assert!(r.linger);
			assert!(r.runtime_dir);
		});
	}

	#[test]
	fn collect_status_reports_absent_unit() {
		with_env(|_root| {
			let sc = FakeCtl::new();
			let r = collect_status(&sc, "nope");
			assert!(!r.unit_exists);
			assert!(r.unit_mode.is_none());
		});
	}

	#[test]
	fn install_surfaces_systemctl_failure() {
		with_env(|root| {
			let mut sc = FakeCtl::new();
			sc.fail = true;
			let err = install(&sc, &opts(root, "app", false, false)).unwrap_err();
			assert!(matches!(err, ComposeError::Autostart(_)));
		});
	}
}
