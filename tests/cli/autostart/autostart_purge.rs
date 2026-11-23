//! `autostart uninstall --purge` tears the stack down and removes its named
//! volumes; without `--purge` it never reaches the engine. `systemctl` is a
//! recording script on `PATH`, so the real user manager is never touched, the
//! config home is a temp dir, and the engine is a fake Unix socket.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
    volumes:
      - data:/data
      - shared:/shared
volumes:
  data: {}
  shared:
    external: true
";

/// A running container of the project, and every volume labelled as its own.
fn project(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("/containers/json") {
		return Some(Reply {
			status: 200,
			body: format!(
				r#"[{{"Id":"{id}","Names":["proj-web-1"],"Image":"web-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"proj","podup.service":"web"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}]"#,
				id = "a".repeat(64),
			),
		});
	}
	if line.starts_with("GET") && line.contains("/volumes/") && line.contains("/json") {
		return Some(Reply {
			status: 200,
			body: r#"{"Name":"proj_data","Labels":{"podup.project":"proj"}}"#.to_string(),
		});
	}
	None
}

/// The workspace: the engine's directory holds the compose file, a `bin` with
/// the recording `systemctl`, and the config home the unit lives under.
struct Workspace {
	engine: FakeEngine,
}

impl Workspace {
	fn new(systemctl_fails_on: Option<&str>) -> Self {
		let engine = FakeEngine::start(project);
		let dir = engine.dir();
		fs::write(dir.join("compose.yaml"), COMPOSE).unwrap();
		fs::create_dir_all(dir.join("bin")).unwrap();
		fs::create_dir_all(dir.join("config")).unwrap();
		// `is-active` answers 4 (unknown unit), which is what an uninstall of a
		// unit systemd never heard of sees. Every call is recorded.
		let fail = systemctl_fails_on.unwrap_or("\u{0}never");
		for tool in ["systemctl", "loginctl"] {
			let script = format!(
				"#!/bin/sh\necho \"{tool} $*\" >> \"{log}\"\n\
				 case \"$*\" in *is-active*) exit 4 ;; *'{fail}'*) echo 'refused' >&2; exit 1 ;; esac\nexit 0\n",
				log = dir.join("systemctl.log").display(),
			);
			let path = dir.join("bin").join(tool);
			fs::write(&path, script).unwrap();
			fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
		}
		Self { engine }
	}

	fn dir(&self) -> &Path {
		self.engine.dir()
	}

	fn uninstall(&self, extra: &[&str]) -> (Option<i32>, String) {
		let path = format!("{}:/usr/bin:/bin", self.dir().join("bin").display());
		let config = self.dir().join("config");
		let mut args = vec!["-f", "compose.yaml", "-p", "proj"];
		args.extend_from_slice(extra);
		args.extend(["autostart", "uninstall"]);
		self.engine.run_with_status(
			None,
			&[
				("PATH", path.as_str()),
				("XDG_CONFIG_HOME", config.to_str().unwrap()),
				("HOME", self.dir().to_str().unwrap()),
			],
			&args,
		)
	}

	fn uninstall_purge(&self) -> (Option<i32>, String) {
		let path = format!("{}:/usr/bin:/bin", self.dir().join("bin").display());
		let config = self.dir().join("config");
		self.engine.run_with_status(
			None,
			&[
				("PATH", path.as_str()),
				("XDG_CONFIG_HOME", config.to_str().unwrap()),
				("HOME", self.dir().to_str().unwrap()),
			],
			&[
				"-f",
				"compose.yaml",
				"-p",
				"proj",
				"autostart",
				"uninstall",
				"--purge",
			],
		)
	}

	fn requests(&self) -> Vec<String> {
		self.engine
			.sent()
			.lines()
			.filter(|l| l.starts_with("GET ") || l.starts_with("POST ") || l.starts_with("DELETE "))
			.map(str::to_string)
			.collect()
	}

	fn systemctl_log(&self) -> String {
		fs::read_to_string(self.dir().join("systemctl.log")).unwrap_or_default()
	}
}

fn deletes(requests: &[String]) -> Vec<&String> {
	requests
		.iter()
		.filter(|l| l.starts_with("DELETE "))
		.collect()
}

#[test]
fn purge_removes_the_container_and_the_declared_volume() {
	let ws = Workspace::new(None);
	let (code, output) = ws.uninstall_purge();
	assert_eq!(code, Some(0), "{output}");
	let requests = ws.requests();
	let deletes = deletes(&requests);
	assert!(
		deletes.iter().any(|l| l.contains("/containers/proj-web-1")),
		"the stack comes down: {requests:?}"
	);
	assert!(
		deletes.iter().any(|l| l.contains("/volumes/proj_data ")),
		"its named volume is removed: {requests:?}"
	);
	assert!(
		requests.iter().all(|l| !l.contains("shared")),
		"an external volume is not the project's: {requests:?}"
	);
}

#[test]
fn purge_also_removes_the_unit() {
	let ws = Workspace::new(None);
	let (code, output) = ws.uninstall_purge();
	assert_eq!(code, Some(0), "{output}");
	assert!(
		ws.systemctl_log().contains("daemon-reload"),
		"the uninstall itself ran: {}",
		ws.systemctl_log()
	);
}

#[test]
fn without_purge_the_engine_is_never_contacted() {
	let ws = Workspace::new(None);
	let (code, output) = ws.uninstall(&[]);
	assert_eq!(code, Some(0), "{output}");
	assert!(
		ws.engine.requests().is_empty(),
		"uninstall alone is not allowed to reach Podman: {:?}",
		ws.requests()
	);
	assert!(
		ws.systemctl_log().contains("daemon-reload"),
		"the uninstall itself ran: {}",
		ws.systemctl_log()
	);
}

/// The unit removal failing must not leave the stack and its volumes behind:
/// the purge runs first and the failure is reported after it.
#[test]
fn purge_still_runs_when_the_unit_removal_fails() {
	let ws = Workspace::new(Some("daemon-reload"));
	let (code, output) = ws.uninstall_purge();
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("daemon-reload"), "{output}");
	let requests = ws.requests();
	assert!(
		deletes(&requests)
			.iter()
			.any(|l| l.contains("/volumes/proj_data ")),
		"the volumes are removed all the same: {requests:?}"
	);
}

#[test]
fn purge_without_a_reachable_engine_fails_and_says_so() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(dir.path().join("compose.yaml"), COMPOSE).unwrap();
	fs::create_dir_all(dir.path().join("bin")).unwrap();
	let tool = dir.path().join("bin").join("systemctl");
	fs::write(
		&tool,
		"#!/bin/sh\ncase \"$*\" in *is-active*) exit 4 ;; esac\nexit 0\n",
	)
	.unwrap();
	fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
	let out = std::process::Command::new(env!("CARGO_BIN_EXE_podup"))
		.current_dir(dir.path())
		.env(
			"PATH",
			format!("{}:/usr/bin:/bin", dir.path().join("bin").display()),
		)
		.env("XDG_CONFIG_HOME", dir.path().join("config"))
		.env("XDG_RUNTIME_DIR", dir.path())
		.env("HOME", dir.path())
		.env("NO_COLOR", "1")
		.env_remove("PODMAN_SOCKET")
		.env_remove("DOCKER_HOST")
		.args(["--socket", "/nonexistent/podman.sock"])
		.args([
			"-f",
			"compose.yaml",
			"-p",
			"proj",
			"autostart",
			"uninstall",
			"--purge",
		])
		.output()
		.unwrap();
	assert!(!out.status.success(), "{out:?}");
	let text = String::from_utf8_lossy(&out.stderr);
	assert!(
		text.contains("/nonexistent/podman.sock") || text.to_lowercase().contains("podman"),
		"the failure names the socket it could not reach: {text}"
	);
}

/// `--dry-run` cannot be honoured by an uninstall, so it is refused before the
/// unit is touched or the engine is asked anything.
#[test]
fn a_dry_run_is_refused_before_anything_is_removed() {
	let ws = Workspace::new(None);
	let path = format!("{}:/usr/bin:/bin", ws.dir().join("bin").display());
	let config = ws.dir().join("config");
	let (code, output) = ws.engine.run_with_status(
		None,
		&[
			("PATH", path.as_str()),
			("XDG_CONFIG_HOME", config.to_str().unwrap()),
			("HOME", ws.dir().to_str().unwrap()),
		],
		&[
			"--dry-run",
			"-f",
			"compose.yaml",
			"-p",
			"proj",
			"autostart",
			"uninstall",
			"--purge",
		],
	);
	assert_eq!(code, Some(2), "{output}");
	assert!(
		output.contains("--dry-run is not supported for `autostart uninstall`"),
		"{output}"
	);
	assert!(
		ws.engine.requests().is_empty(),
		"nothing may reach the engine: {:?}",
		ws.requests()
	);
	assert!(
		ws.systemctl_log().is_empty(),
		"the unit manager must not be asked anything: {}",
		ws.systemctl_log()
	);
}
