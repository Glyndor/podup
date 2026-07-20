//! Unit tests for the service-mode autostart install/uninstall/status logic.
//!
//! Split out of `mod.rs` to keep that file within the source line limit, the
//! same way the quadlet-mode tests live in `quadlet/tests.rs`.

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
