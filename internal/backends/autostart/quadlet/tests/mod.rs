use super::{container_services, install_quadlet, rebuild_quadlet, uninstall_quadlet};
use crate::autostart::SystemCtl;
use crate::{parse_str, quadlet};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ExitStatus, Output};

/// Records every `systemctl` arg vector; `loginctl` always reports linger on.
struct FakeCtl {
	calls: RefCell<Vec<Vec<String>>>,
	states: RefCell<BTreeMap<String, (&'static str, &'static str)>>,
	stop_result: Cell<&'static str>,
	cleanup_status: Cell<i32>,
	resources: Cell<bool>,
}
impl FakeCtl {
	fn new() -> Self {
		FakeCtl {
			calls: RefCell::new(Vec::new()),
			states: RefCell::new(BTreeMap::new()),
			stop_result: Cell::new("success"),
			cleanup_status: Cell::new(0),
			resources: Cell::new(false),
		}
	}
	fn failed(&self, service: &str) {
		self.states
			.borrow_mut()
			.insert(service.to_string(), ("failed", "exit-code"));
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
		let mut states = self.states.borrow_mut();
		let state = states
			.entry(args.last().unwrap().to_string())
			.or_insert(("inactive", "success"));
		match args[0] {
			// systemd does not execute hooks again for an already failed service.
			"stop" if state.0 != "failed" => {
				*state = (
					if self.stop_result.get() == "success" {
						"inactive"
					} else {
						"failed"
					},
					self.stop_result.get(),
				);
			}
			"reset-failed" => *state = ("inactive", "success"),
			_ => {}
		}
		Ok(Output {
			status: ExitStatus::from_raw(0),
			stdout: if args.starts_with(&["show", "--property=ActiveState,Result"]) {
				format!("ActiveState={}\nResult={}\n", state.0, state.1).into_bytes()
			} else if args.starts_with(&["show", "--property=Result", "--value"]) {
				format!("{}\n", state.1).into_bytes()
			} else {
				Vec::new()
			},
			stderr: Vec::new(),
		})
	}
	fn cleanup(&self, command: &str) -> std::io::Result<Output> {
		self.calls
			.borrow_mut()
			.push(vec!["cleanup".to_string(), command.to_string()]);
		if self.cleanup_status.get() == 0 {
			self.resources.set(false);
		}
		Ok(Output {
			status: ExitStatus::from_raw(self.cleanup_status.get() << 8),
			stdout: Vec::new(),
			stderr: b"remaining own-resource".to_vec(),
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

/// A [`FakeCtl`] whose exit status is decided per call, so a test can make one
/// specific `systemctl` verb fail while the rest succeed. `FakeCtl` always
/// reports 0, which cannot express "this service is not running" (`is-active`
/// non-zero) or "the stop was refused".
struct ScriptedCtl {
	calls: RefCell<Vec<Vec<String>>>,
	/// Exit code for a given arg vector; 0 means success.
	code: ExitCodeFor,
	/// Post-stop ActiveState/Result response; None simulates a query launch failure.
	result: Option<&'static [u8]>,
}

/// Decides a scripted `systemctl` call's exit code from its arguments.
type ExitCodeFor = Box<dyn Fn(&[&str]) -> i32>;
impl ScriptedCtl {
	fn new(code: impl Fn(&[&str]) -> i32 + 'static) -> Self {
		ScriptedCtl {
			calls: RefCell::new(Vec::new()),
			code: Box::new(code),
			result: Some(b"ActiveState=inactive\nResult=success\n"),
		}
	}
	fn log(&self) -> Vec<Vec<String>> {
		self.calls.borrow().clone()
	}
}
impl SystemCtl for ScriptedCtl {
	fn systemctl(&self, args: &[&str]) -> std::io::Result<Output> {
		self.calls
			.borrow_mut()
			.push(args.iter().map(|s| s.to_string()).collect());
		let stdout = if args.starts_with(&["show", "--property=ActiveState,Result"]) {
			let stopped = self
				.calls
				.borrow()
				.iter()
				.any(|call| call == &["stop", args[2]]);
			if stopped {
				self.result
					.ok_or_else(|| std::io::Error::other("query unavailable"))?
					.to_vec()
			} else {
				b"ActiveState=inactive\nResult=success\n".to_vec()
			}
		} else {
			Vec::new()
		};
		Ok(Output {
			status: ExitStatus::from_raw((self.code)(args) << 8),
			stdout,
			stderr: b"scripted failure".to_vec(),
		})
	}
	fn cleanup(&self, command: &str) -> std::io::Result<Output> {
		self.calls
			.borrow_mut()
			.push(vec!["cleanup".to_string(), command.to_string()]);
		Ok(Output {
			status: ExitStatus::from_raw((self.code)(&["cleanup", command]) << 8),
			stdout: Vec::new(),
			stderr: b"scripted cleanup failure".to_vec(),
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

pub(super) const IMG: &str = "services:\n  web:\n    image: nginx\n";
pub(super) const BUILD: &str = "services:\n  web:\n    build: .\n";
const IMG_MULTI: &str =
	"services:\n  web:\n    image: nginx\n  api:\n    image: nginx\n  worker:\n    image: nginx\n";
const BASE: &str = "/srv/app";

mod failed_cleanup;
mod install;
mod oversized;
mod ownership;
mod rebuild;
mod secret_paths;
mod stop_result;
mod uninstall;
// #1970: prebuilt-shape install + rebuild behaviour. Split into its own
// module so the per-file code-line cap stays under 500.
#[cfg(all(test, unix))]
mod build;
