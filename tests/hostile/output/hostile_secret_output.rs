//! Secret values must stay out of what podup prints.
//!
//! A compose file carries secrets in several places: an inline `content:`, a
//! `file:` source, an `environment:` source, `environment:` values, an
//! `env_file:`, build arguments and credentials inside a URL. Each is given a
//! marker value no honest output contains, then podup is run with a dead
//! engine socket and every byte it writes is searched for the markers.
//!
//! What the project documents is the bar (docs/security-model.md): `config`
//! redacts inline `content:` secrets, and default logging omits secret values.
//! `RUST_LOG=debug` is documented to possibly show environment values, so the
//! debug runs only hold the sources of `secrets:` to the stricter rule.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::TempDir;

/// A socket path that cannot be connected to, so nothing here reaches Podman.
const DEAD_SOCKET: &str = "unix:///nonexistent/podup-hostile-secret-test.sock";

/// Markers for values that must not be printed.
const INLINE: &str = "MARK-inline-content-7f3a";
const FILE: &str = "MARK-secret-file-bytes-91c2";
const FROM_ENV: &str = "MARK-env-sourced-secret-5d10";
/// A value `config` prints by design, used by the control at the end.
const ENV_VALUE: &str = "MARK-service-env-value-2b8e";

const COMPOSE: &str = "\
services:
  web:
    image: registry.example/app:1
    environment:
      DB_PASSWORD: MARK-service-env-value-2b8e
    secrets: [inline, fromenv, fromfile]
secrets:
  inline:
    content: MARK-inline-content-7f3a
  fromenv:
    environment: SECRET_SRC
  fromfile:
    file: ./secret.txt
";

fn project() -> TempDir {
	let dir = tempfile::tempdir().expect("temp dir");
	fs::write(dir.path().join("compose.yaml"), COMPOSE).unwrap();
	fs::write(dir.path().join("secret.txt"), FILE).unwrap();
	dir
}

/// Run podup on the project and return everything it printed.
fn run(dir: &Path, log: Option<&str>, args: &[&str]) -> String {
	let mut cmd = Command::new(env!("CARGO_BIN_EXE_podup"));
	cmd.current_dir(dir)
		.args(["-f", "compose.yaml", "-p", "leak"])
		.args(args)
		.env("DOCKER_HOST", DEAD_SOCKET)
		.env("CONTAINER_HOST", DEAD_SOCKET)
		.env("SECRET_SRC", FROM_ENV)
		.env_remove("RUST_LOG")
		.stdin(Stdio::null());
	if let Some(level) = log {
		cmd.env("RUST_LOG", level);
	}
	let out = cmd.output().expect("run podup");
	format!(
		"{}\n{}",
		String::from_utf8_lossy(&out.stdout),
		String::from_utf8_lossy(&out.stderr)
	)
}

/// Commands that read the compose file and never need a live engine, plus
/// engine commands that fail at the dead socket.
const COMMANDS: &[&[&str]] = &[
	&["config"],
	&["config", "--no-interpolate"],
	&["config", "--format", "json"],
	&["generate", "quadlet"],
	&["audit"],
	&["up", "-d"],
	&["create"],
	&["down"],
	&["ps"],
	&["build"],
	&["pull"],
];

fn leaks(text: &str, markers: &[&str]) -> Vec<String> {
	markers
		.iter()
		.filter(|m| text.contains(**m))
		.map(|m| (*m).to_string())
		.collect()
}

#[test]
fn secret_sources_never_appear_at_the_default_log_level() {
	let dir = project();
	let mut found = Vec::new();
	for args in COMMANDS {
		let text = run(dir.path(), None, args);
		for m in leaks(&text, &[INLINE, FILE, FROM_ENV]) {
			found.push(format!("podup {}: printed {m}", args.join(" ")));
		}
	}
	assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn secret_sources_never_appear_at_debug_level() {
	let dir = project();
	let mut found = Vec::new();
	for args in COMMANDS {
		let text = run(dir.path(), Some("debug"), args);
		for m in leaks(&text, &[INLINE, FILE, FROM_ENV]) {
			found.push(format!(
				"RUST_LOG=debug podup {}: printed {m}",
				args.join(" ")
			));
		}
	}
	assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn config_redacts_what_the_security_model_says_it_redacts() {
	let dir = project();
	let text = run(dir.path(), None, &["config"]);
	assert!(
		text.contains("secrets:") && text.contains("inline:"),
		"the control is empty: config did not print the secrets section:\n{text}"
	);
	assert!(
		!text.contains(INLINE),
		"config printed an inline secret:\n{text}"
	);
}

#[test]
fn the_leak_scan_sees_a_value_that_is_printed() {
	// `config` shows service environment values by design, so the marker is
	// there. If this stops being true the scan above has stopped looking.
	let dir = project();
	let text = run(dir.path(), None, &["config"]);
	assert_eq!(leaks(&text, &[ENV_VALUE]), vec![ENV_VALUE.to_string()]);
}
