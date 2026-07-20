//! `podup autostart` service mode: a single rootless `systemctl --user` unit that
//! brings a compose stack up at boot.
//!
//! Everything here is user-scope only — the unit lives under
//! `${XDG_CONFIG_HOME:-~/.config}/systemd/user/` and every action goes through
//! `systemctl --user` / `loginctl`. No root, no `sudo`, nothing under `/etc` or
//! the system systemd. External-command calls go through the `SystemCtl` seam so
//! the install/uninstall/status logic is unit-testable without a live systemd.

mod quadlet;
mod service;

pub use quadlet::{install_quadlet, rebuild_quadlet, uninstall_quadlet};
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

	// Fail closed on values a unit line cannot represent (control characters
	// would inject directives via the literal `WorkingDirectory=` line).
	service::validate_unit_opts(&opts.unit).map_err(ComposeError::Autostart)?;

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

/// Whether systemd knows anything about `unit` — loaded, enabled, running, or
/// merely present as a fragment.
///
/// `systemctl is-active` exits **4** for a unit it has never heard of and
/// something else for every other state (0 active, 3 inactive/failed/activating).
/// That numeric 4 is the only reliable "there is nothing here" signal: the
/// message text is localised, and the *fragment file* is not a proxy for it —
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
/// Idempotent — uninstalling when nothing is installed is a quiet no-op — but a
/// `disable` that genuinely fails is reported rather than swallowed, so the
/// command cannot claim success while the service is still enabled and running.
pub fn uninstall<S: SystemCtl>(sc: &S, project: &str) -> crate::Result<()> {
	let unit_name = unit_file_name(project);
	let path = unit_path(project);

	// Disable whenever systemd knows the unit, whether or not its file is still
	// there. `disable --now` is idempotent across every state it can be in
	// (enabled, never enabled, running, stopped, fragment deleted out of band),
	// so the only case worth skipping is the one where systemd has never heard
	// of it — which is exactly what `unit_is_known` answers.
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

	checked(sc.systemctl(&["daemon-reload"]), "daemon-reload")?;
	Ok(())
}

/// Which autostart mode, if any, is installed for a project. Service and quadlet
/// mode cannot coexist — each install refuses the other — so at most one is present.
/// `uninstall` uses this to remove whichever is there without the caller naming a
/// mode (and mistakenly no-op'ing against the wrong one).
pub enum InstalledMode {
	/// The service-mode `podup-<project>.service` unit is present.
	Service,
	/// Quadlet `<project>-*.container` units are present.
	Quadlet,
	/// Neither — nothing is installed.
	None,
}

/// Detect the installed autostart mode for `project` from what is on disk.
pub fn installed_mode(project: &str) -> InstalledMode {
	if unit_path(project).exists() {
		InstalledMode::Service
	} else if !quadlet_units_present(project).is_empty() {
		InstalledMode::Quadlet
	} else {
		InstalledMode::None
	}
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
		/// Exit code for `is-active`, as a raw wait status (code << 8). 0 by
		/// default; `4 << 8` is systemd's "no such unit", the only value
		/// `unit_is_known` treats as "there is nothing here".
		is_active_code: i32,
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
				is_active_code: 0,
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
			// `is-enabled` is read through stdout only, so its status stays
			// successful. `is-active`'s status *is* consulted (`unit_is_known`
			// keys off exit 4), so it comes from its own field rather than the
			// blanket `fail` flag.
			let code = match args.first().copied() {
				Some("is-active") => self.is_active_code,
				Some("is-enabled") => 0,
				_ => code,
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
			// The `is-active` probe only asks whether systemd knows the unit at
			// all; anything but exit 4 means disable it.
			assert_eq!(calls[0], vec!["is-active", "--quiet", "podup-app.service"]);
			assert_eq!(calls[1], vec!["disable", "--now", "podup-app.service"]);
			assert_eq!(calls[2], vec!["daemon-reload"]);
		});
	}

	/// #1080: a `disable --now` that fails on an installed unit was swallowed by
	/// `let _ =`, so uninstall exited 0 with the service still enabled and
	/// running. Measured against real systemd: with the unit file present,
	/// `disable --now` exits 0 whether or not the unit was ever enabled or
	/// started, so a non-zero exit here is always a real failure.
	#[test]
	fn uninstall_reports_a_failed_disable() {
		with_env(|root| {
			install(&FakeCtl::new(), &opts(root, "app", false, true)).unwrap();
			let path = root.join("systemd/user/podup-app.service");
			assert!(path.exists());

			let mut sc = FakeCtl::new();
			sc.fail = true;
			let err = uninstall(&sc, "app")
				.expect_err("a failed disable must not be reported as a clean uninstall");
			assert!(matches!(err, ComposeError::Autostart(_)), "got {err:?}");
			assert!(err.to_string().contains("disable"), "got {err}");
		});
	}

	/// Uninstalling when systemd has never heard of the unit stays a silent
	/// no-op. `is-active` exit 4 ("no such unit") is the signal — not the unit
	/// file, which is a poor proxy: a fragment deleted out of band leaves the
	/// unit loaded, enabled and running, and only `disable --now` clears it.
	#[test]
	fn uninstall_runs_no_disable_when_systemd_does_not_know_the_unit() {
		with_env(|_root| {
			let mut sc = FakeCtl::new();
			sc.is_active_code = 4 << 8; // systemd: no such unit
			uninstall(&sc, "app").expect("uninstalling nothing is not a failure");
			let calls = sc.systemctl_log();
			assert!(
				!calls
					.iter()
					.any(|c| c.first().map(String::as_str) == Some("disable")),
				"nothing is installed, so nothing should be disabled: {calls:?}"
			);
			assert_eq!(calls.last().unwrap(), &vec!["daemon-reload".to_string()]);
		});
	}

	/// The mirror case, and the reason the file is not the gate: the unit file is
	/// gone but systemd still has the unit loaded and running (a manual `rm`, a
	/// restored `~/.config`, a half-finished uninstall). `disable --now` is the
	/// only thing that clears that state, so it must still run.
	#[test]
	fn uninstall_disables_a_known_unit_whose_file_is_already_gone() {
		with_env(|root| {
			let path = root.join("systemd/user/podup-app.service");
			assert!(!path.exists());
			// Default `is_active_code` is 0 — systemd knows the unit.
			let sc = FakeCtl::new();
			uninstall(&sc, "app").expect("uninstall must still succeed");
			let calls = sc.systemctl_log();
			assert!(
				calls.contains(&vec![
					"disable".to_string(),
					"--now".to_string(),
					"podup-app.service".to_string()
				]),
				"a unit systemd still knows must be disabled even with no file: {calls:?}"
			);
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
