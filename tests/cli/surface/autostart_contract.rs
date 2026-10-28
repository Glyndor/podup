//! CLI-surface checks for `podup autostart install`: what the generated unit
//! carries, and which services it covers.
//!
//! Every scenario runs with `--dry-run`, so nothing is installed and no unit
//! is written; the checks read the unit the command prints. The config home
//! and the runtime directory point at temp dirs, so the real HOME is never
//! touched, and the fake `podman` on `PATH` is a file the command only looks
//! up: it is never run.
//!
//! `autostart` writes systemd units, so these checks run on unix only.

#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// A compose file with one plain service and one behind the `debug` profile.
const PROFILED: &str =
	"services:\n  web:\n    image: nginx\n  debug:\n    image: nginx\n    profiles: [debug]\n";

/// A compose file with a single service.
const PLAIN: &str = "services:\n  web:\n    image: nginx\n";

/// A temp workspace holding `compose.yml`, and the temp home the command must
/// use instead of the real one.
struct Workspace {
	dir: tempfile::TempDir,
	home: tempfile::TempDir,
}

impl Workspace {
	fn new(compose: &str) -> Self {
		let dir = tempfile::tempdir().expect("tempdir");
		fs::write(dir.path().join("compose.yml"), compose).expect("write compose");
		Self {
			dir,
			home: tempfile::tempdir().expect("tempdir"),
		}
	}

	/// Run `podup autostart install --dry-run` with the given extra
	/// arguments, inside the workspace and with the temp home.
	fn install(&self, extra: &[&str]) -> Output {
		let mut cmd = Command::new(env!("CARGO_BIN_EXE_podup"));
		cmd.current_dir(self.dir.path())
			.env("XDG_CONFIG_HOME", self.home.path())
			.env("XDG_RUNTIME_DIR", self.home.path())
			.env_remove("PODMAN_SOCKET")
			.env_remove("DOCKER_HOST")
			.env_remove("COMPOSE_PROFILES")
			.args([
				"-f",
				"compose.yml",
				"-p",
				"proj",
				"autostart",
				"install",
				"--dry-run",
			]);
		cmd.args(extra);
		cmd.output().expect("run podup")
	}
}

/// A `podman` on `PATH` for the start mode, which needs its absolute path and
/// never runs it.
fn fake_podman() -> tempfile::TempDir {
	let dir = tempfile::tempdir().expect("tempdir");
	let path = dir.path().join("podman");
	fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write podman");
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
	}
	dir
}

/// The path the command prints, with the temp dir resolved the way the command
/// resolves it.
fn resolved(dir: &Path) -> String {
	std::fs::canonicalize(dir)
		.unwrap_or_else(|_| dir.to_path_buf())
		.display()
		.to_string()
}

/// The unit the service-mode dry run prints.
#[test]
fn the_service_unit_carries_the_selected_socket_and_project_directory() {
	let ws = Workspace::new(PLAIN);
	let socket = "/run/user/1000/podman/podman.sock";
	let out = ws.install(&[
		"--mode",
		"service",
		"--socket",
		socket,
		"--project-directory",
		ws.dir.path().to_str().expect("a path"),
	]);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		out.status.success(),
		"the dry run must succeed; stderr: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	assert!(
		stdout.contains(&format!("--socket {socket}")),
		"the selected socket must travel into the unit:\n{stdout}"
	);
	assert!(
		stdout.contains(&format!("--project-directory {}", resolved(ws.dir.path()))),
		"the selected project directory must travel into the unit:\n{stdout}"
	);
}

/// A service behind an inactive profile is not part of the project, so quadlet
/// mode must not write a unit for it.
#[test]
fn the_quadlet_units_cover_only_the_active_services() {
	let ws = Workspace::new(PROFILED);
	let out = ws.install(&["--mode", "quadlet"]);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		out.status.success(),
		"the dry run must succeed; stderr: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	assert!(
		stdout.contains("proj-web.container"),
		"the active service must have its unit:\n{stdout}"
	);
	assert!(
		!stdout.contains("proj-debug.container"),
		"the service behind an inactive profile must not have a unit:\n{stdout}"
	);
}

/// Start mode resolves the one container the project has. A project whose
/// other service sits behind an inactive profile still has exactly one active
/// service, and must not be refused for having two.
#[test]
fn the_start_unit_names_the_container_the_active_profiles_select() {
	let ws = Workspace::new(PROFILED);
	let podman = fake_podman();
	let path = format!(
		"{}:{}",
		podman.path().display(),
		std::env::var("PATH").unwrap_or_default()
	);
	let mut cmd = Command::new(env!("CARGO_BIN_EXE_podup"));
	cmd.current_dir(ws.dir.path())
		.env("XDG_CONFIG_HOME", ws.home.path())
		.env("XDG_RUNTIME_DIR", ws.home.path())
		.env("PATH", path)
		.env_remove("PODMAN_SOCKET")
		.env_remove("DOCKER_HOST")
		.env_remove("COMPOSE_PROFILES")
		.args([
			"-f",
			"compose.yml",
			"-p",
			"proj",
			"autostart",
			"install",
			"--mode",
			"start",
			"--dry-run",
		]);
	let out = cmd.output().expect("run podup");
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		out.status.success(),
		"the dry run must succeed; stderr: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	assert!(
		stdout.contains("proj-web-1"),
		"the unit must name the container of the active service:\n{stdout}"
	);
	assert!(
		!stdout.contains("proj-debug-1"),
		"the unit must not name the container of an inactive service:\n{stdout}"
	);
}
