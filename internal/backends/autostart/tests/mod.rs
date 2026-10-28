//! Unit tests for the service-mode autostart install/uninstall/status logic.
//!
//! Split out of `mod.rs` to keep that file within the source line limit, the
//! same way the quadlet-mode tests live in `quadlet/tests.rs`.

use super::guards::{linger_warning, runtime_dir_warning};
use super::install::{install, uninstall};
use super::options::InstallOptions;
use super::paths::max_stop_grace_secs;
use super::service::{render_service_unit, render_update_timer_unit, ServiceUnitOpts};
use super::start::{render_start_unit, StartUnitOpts};
use super::status::{collect_status, NetworkWait, NETWORK_SHIM};
use super::systemctl::SystemCtl;
use crate::ComposeError;
use std::cell::RefCell;
use std::io;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::{ExitStatus, Output};

/// Recording fake: captures every `systemctl`/`loginctl` arg vector and returns
/// canned output keyed off the first argument.
pub(super) struct FakeCtl {
	systemctl_calls: RefCell<Vec<Vec<String>>>,
	loginctl_calls: RefCell<Vec<Vec<String>>>,
	linger: String,
	is_active: String,
	is_enabled: String,
	/// Exit code for `is-active`, as a raw wait status (code << 8). 0 by
	/// default; `4 << 8` is systemd's "no such unit", the only value
	/// `unit_is_known` treats as "there is nothing here".
	is_active_code: i32,
	/// What `systemctl --user show <shim> -p LoadState` prints. Real systemd
	/// exits 0 whether or not the unit exists, so this fake always does too:
	/// a fake that signalled absence through the exit code would let a guard
	/// that reads the code pass here and fail in production.
	show_load_state: String,
	fail: bool,
}

impl FakeCtl {
	pub(super) fn new() -> Self {
		FakeCtl {
			systemctl_calls: RefCell::new(Vec::new()),
			loginctl_calls: RefCell::new(Vec::new()),
			linger: "yes".to_string(),
			is_active: "active".to_string(),
			is_enabled: "enabled".to_string(),
			is_active_code: 0,
			show_load_state: "LoadState=loaded\n".to_string(),
			fail: false,
		}
	}

	pub(super) fn systemctl_log(&self) -> Vec<Vec<String>> {
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
			Some("show") => self.show_load_state.as_str(),
			_ => "",
		};
		// `is-enabled` is read through stdout only, so its status stays
		// successful. `is-active`'s status *is* consulted (`unit_is_known`
		// keys off exit 4), so it comes from its own field rather than the
		// blanket `fail` flag.
		let code = match args.first().copied() {
			Some("is-active") => self.is_active_code,
			// Both 0, deliberately. Measured on real systemd: `show` reports a
			// missing unit through `LoadState=not-found` in stdout and still
			// exits 0, unlike `is-enabled`, which exits 4 for the same unit.
			Some("is-enabled") | Some("show") => 0,
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

pub(super) fn opts(dir: &Path, project: &str, dry_run: bool, no_start: bool) -> InstallOptions {
	InstallOptions {
		unit: ServiceUnitOpts {
			exe: PathBuf::from("/usr/local/bin/podup"),
			compose_files: vec![dir.join("docker-compose.yml")],
			project: project.to_string(),
			working_dir: dir.to_path_buf(),
			profiles: Vec::new(),
			env_files: Vec::new(),
			max_stop_grace_secs: None,
			socket: None,
			project_directory: None,
		},
		no_start,
		dry_run,
		auto_update_interval: None,
	}
}

pub(super) fn opts_with_interval(
	dir: &Path,
	project: &str,
	dry_run: bool,
	no_start: bool,
	interval: &str,
) -> InstallOptions {
	let mut o = opts(dir, project, dry_run, no_start);
	o.auto_update_interval = Some(interval.to_string());
	o
}

/// Run `f` with a fresh temp `XDG_CONFIG_HOME`, `USER`, and `XDG_RUNTIME_DIR`
/// set, so the install/status paths resolve under the temp dir.
pub(super) fn with_env<R>(f: impl FnOnce(&Path) -> R) -> R {
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

mod auto_update;
mod install;
mod status;
mod uninstall;
