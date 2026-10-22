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

/// A [`FakeCtl`] whose exit status is decided per call, so a test can make one
/// specific `systemctl` verb fail while the rest succeed. `FakeCtl` always
/// reports 0, which cannot express "this service is not running" (`is-active`
/// non-zero) or "the stop was refused".
struct ScriptedCtl {
	calls: RefCell<Vec<Vec<String>>>,
	/// Exit code for a given arg vector; 0 means success.
	code: ExitCodeFor,
}

/// Decides a scripted `systemctl` call's exit code from its arguments.
type ExitCodeFor = Box<dyn Fn(&[&str]) -> i32>;
impl ScriptedCtl {
	fn new(code: impl Fn(&[&str]) -> i32 + 'static) -> Self {
		ScriptedCtl {
			calls: RefCell::new(Vec::new()),
			code: Box::new(code),
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
		Ok(Output {
			status: ExitStatus::from_raw((self.code)(args) << 8),
			stdout: Vec::new(),
			stderr: b"scripted failure".to_vec(),
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

mod install;
mod rebuild;
mod uninstall;
// #1970: prebuilt-shape install + rebuild behaviour. Split into its own
// module so the per-file code-line cap stays under 500.
#[cfg(all(test, unix))]
mod build;
