//! Host paths whose bytes are not UTF-8 are legal on Unix, and a compose
//! project can sit in one, or hold a file in one. podup must neither panic on
//! them nor fail without naming what it could not use. The engine is a fake
//! Unix socket, so nothing here touches Podman.

#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[path = "../../support/fake_engine.rs"]
mod fake_engine;
#[path = "../../support/host_files.rs"]
mod host_files;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: x
    build: .
    env_file: .env
    volumes:
      - ./data:/data
";

fn accepts(_: &str, _: &str) -> Option<Reply> {
	None
}

/// A project in a directory whose name is not UTF-8, holding an env file, a bind
/// source, a Dockerfile and a build context file whose name is not UTF-8 either.
struct Project {
	engine: FakeEngine,
	dir: PathBuf,
}

impl Project {
	fn new() -> Self {
		let engine = FakeEngine::start(accepts);
		let dir = engine.dir().join(OsStr::from_bytes(b"proj-\xff"));
		std::fs::create_dir_all(dir.join("data")).unwrap();
		std::fs::write(dir.join(".env"), "A=1\n").unwrap();
		std::fs::write(dir.join("Dockerfile"), "FROM scratch\n").unwrap();
		std::fs::write(dir.join(OsStr::from_bytes(b"ctx-\xfe.txt")), "x").unwrap();
		std::fs::write(dir.join("compose.yaml"), COMPOSE).unwrap();
		Self { engine, dir }
	}

	fn run(&self, args: &[&OsStr]) -> Output {
		self.run_with_env(args, &[])
	}

	fn run_with_env(&self, args: &[&OsStr], env: &[(&str, &OsStr)]) -> Output {
		let mut cmd = Command::new(env!("CARGO_BIN_EXE_podup"));
		cmd.current_dir(&self.dir)
			.arg("--socket")
			.arg(self.engine.dir().join("podman.sock"))
			.args(args)
			.env("NO_COLOR", "1")
			.env_remove("RUST_LOG")
			.env_remove("COMPOSE_FILE")
			.stdin(Stdio::null());
		for (key, value) in env {
			cmd.env(key, value);
		}
		cmd.output().expect("run podup")
	}

	fn run_str(&self, args: &[&str]) -> Output {
		let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
		self.run(&args)
	}
}

fn stderr(out: &Output) -> String {
	String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The process ended by its own decision, with no panic report.
fn assert_no_crash(what: &str, out: &Output) {
	let err = stderr(out);
	assert_eq!(
		out.status.signal(),
		None,
		"{what} was ended by a signal: {err}"
	);
	assert_ne!(out.status.code(), Some(101), "{what} panicked: {err}");
	for marker in ["internal error", "panicked", "looks like a bug"] {
		assert!(!err.contains(marker), "{what} reported {marker:?}: {err}");
	}
}

#[test]
fn a_compose_file_whose_name_is_not_utf8_is_read_like_any_other() {
	let project = Project::new();
	let file: PathBuf = project.dir.join(OsStr::from_bytes(b"compose-\xff.yaml"));
	std::fs::copy(project.dir.join("compose.yaml"), &file).unwrap();
	let file: &Path = &file;

	for args in [&["config"][..], &["up", "--no-start"][..]] {
		let mut full: Vec<&OsStr> = vec![OsStr::new("-f"), file.as_os_str()];
		full.extend(args.iter().map(OsStr::new));
		let out = project.run(&full);
		assert_no_crash(&args.join(" "), &out);
		assert_eq!(
			out.status.code(),
			Some(0),
			"`{}`: {}",
			args.join(" "),
			stderr(&out)
		);
	}
}

#[test]
fn a_project_directory_that_is_not_utf8_resolves_its_env_file_bind_source_and_context() {
	let project = Project::new();

	let config = project.run_str(&["-f", "compose.yaml", "config"]);
	assert_no_crash("config", &config);
	assert_eq!(config.status.code(), Some(0), "{}", stderr(&config));
	let printed = String::from_utf8_lossy(&config.stdout);
	assert!(
		printed.contains("A: '1'"),
		"the env file was not read:\n{printed}"
	);
	assert!(
		printed.contains("/data:/data"),
		"the bind source was not resolved:\n{printed}"
	);

	for args in [&["build"][..], &["up", "--no-start", "--build"][..]] {
		let mut full = vec!["-f", "compose.yaml"];
		full.extend_from_slice(args);
		let out = project.run_str(&full);
		assert_no_crash(&args.join(" "), &out);
		assert_eq!(
			out.status.code(),
			Some(0),
			"`{}`: {}",
			args.join(" "),
			stderr(&out)
		);
	}
}

#[test]
fn a_build_context_file_whose_name_is_not_utf8_is_shipped_or_named_in_the_refusal() {
	let project = Project::new();
	let out = project.run_str(&["-f", "compose.yaml", "build"]);
	assert_no_crash("build", &out);

	let name = b"ctx-\xfe.txt";
	let shipped = project
		.engine
		.requests()
		.iter()
		.filter(|r| r.starts_with(b"POST"))
		.map(|r| host_files::partial_archive_bytes(r))
		.any(|tar| tar.windows(name.len()).any(|w| w == name));
	let named = stderr(&out).contains("ctx-");
	assert!(
		shipped || (out.status.code() != Some(0) && named),
		"the file was neither sent in the context nor named in an error ({:?}): {}",
		out.status.code(),
		stderr(&out)
	);
}

#[test]
fn a_compose_file_with_invalid_bytes_is_refused_with_its_name() {
	let project = Project::new();
	std::fs::write(
		project.dir.join("broken.yaml"),
		b"services:\n  web:\n    image: \xff\n",
	)
	.unwrap();
	let out = project.run_str(&["-f", "broken.yaml", "config"]);
	assert_no_crash("config", &out);
	assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
	assert!(
		stderr(&out).contains("broken.yaml"),
		"the refusal does not name the file: {}",
		stderr(&out)
	);
}

#[test]
fn a_compose_file_variable_that_is_not_utf8_is_ignored_without_a_panic() {
	let project = Project::new();
	let bad = OsStr::from_bytes(b"compose-\xff.yaml");
	let out = project.run_with_env(&[OsStr::new("config")], &[("COMPOSE_FILE", bad)]);
	assert_no_crash("config", &out);
	// Like any variable that is not text, it cannot name a file, so the default
	// compose file is the one read.
	assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn an_argument_that_cannot_be_text_is_refused_with_a_usage_error() {
	let project = Project::new();
	let bad = OsStr::from_bytes(b"\xff");
	let compose = OsStr::new("compose.yaml");
	for args in [
		vec![OsStr::new("-f"), compose, OsStr::new("logs"), bad],
		vec![OsStr::new("-f"), compose, OsStr::new("up"), bad],
		vec![
			OsStr::new("-f"),
			compose,
			OsStr::new("exec"),
			OsStr::new("web"),
			bad,
		],
		vec![
			OsStr::new("-f"),
			compose,
			OsStr::new("-p"),
			bad,
			OsStr::new("ps"),
		],
		vec![
			OsStr::new("-f"),
			compose,
			OsStr::new("--env-file"),
			bad,
			OsStr::new("config"),
		],
		vec![
			OsStr::new("-f"),
			compose,
			OsStr::new("--profile"),
			bad,
			OsStr::new("config"),
		],
	] {
		let out = project.run(&args);
		assert_no_crash(&format!("{args:?}"), &out);
		assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
	}
}
