use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub(super) struct Workspace {
	pub root: tempfile::TempDir,
	pub config: PathBuf,
}

impl Workspace {
	pub fn new() -> Self {
		let root = tempfile::tempdir().unwrap();
		fs::write(
			root.path().join("compose.yml"),
			"services:\n  web:\n    image: alpine\n",
		)
		.unwrap();
		let config = root.path().join("config");
		Self { root, config }
	}

	pub fn run(&self, args: &[&str]) -> Output {
		// Change umask only in the child; parallel tests keep their process state.
		Command::new("sh")
			.args([
				"-c",
				"umask 000; exec \"$@\"",
				"podup-test",
				env!("CARGO_BIN_EXE_podup"),
			])
			.current_dir(self.root.path())
			.env("XDG_CONFIG_HOME", &self.config)
			.env("XDG_RUNTIME_DIR", self.root.path())
			.env("HOME", self.root.path())
			.env("RUST_LOG", "off")
			.env("NO_COLOR", "1")
			.env_remove("PODMAN_SOCKET")
			.env_remove("DOCKER_HOST")
			.env_remove("COMPOSE_PROFILES")
			.args(["-f", "compose.yml", "-p", "f6"])
			.args(args)
			.output()
			.unwrap()
	}

	pub fn install(&self, mode: &str) -> Output {
		self.run(&["autostart", "install", "--mode", mode, "--no-start"])
	}

	pub fn export(&self, dir: &Path) -> Output {
		self.run(&["generate", "quadlet", "-o", dir.to_str().unwrap()])
	}
}

pub(super) fn mode(path: &Path) -> u32 {
	fs::metadata(path).unwrap().permissions().mode() & 0o777
}

pub(super) fn chmod(path: &Path, bits: u32) {
	fs::set_permissions(path, fs::Permissions::from_mode(bits)).unwrap();
}

pub(super) fn stderr(output: &Output) -> String {
	String::from_utf8_lossy(&output.stderr).into_owned()
}
