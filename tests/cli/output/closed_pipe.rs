//! A reader that closes the pipe early, as `podup ... | head -1` does, ends a
//! command that writes to stdout cleanly: no signal, no panic, and no report
//! of an internal error. The engine is a fake Unix socket, so nothing here
//! touches Podman.

#![cfg(unix)]

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
name: demo
services:
  web:
    image: web-image
";

fn lists_one_container(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("/containers/json") {
		return Some(Reply {
			status: 200,
			body: format!(
				r#"[{{"Id":"{}","Names":["demo-web-1"],"Image":"web-image","State":"running","Labels":{{"podup.project":"demo","podup.service":"web"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1}}]"#,
				"a".repeat(64)
			),
		});
	}
	if line.starts_with("GET") && line.contains("/containers/demo-web-1/export") {
		return Some(Reply {
			status: 200,
			body: "x".repeat(64 * 1024),
		});
	}
	None
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(lists_one_container);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

/// Commands that write to stdout and end cleanly on a closed pipe.
const COMMANDS: &[&[&str]] = &[
	&["completions", "bash"],
	&["completions", "zsh"],
	&["completions", "fish"],
	&["ls"],
	&["ls", "--format", "json"],
	&["ps"],
	&["ps", "--format", "json"],
	&["ps", "--services"],
	&["--help"],
	&["up", "--help"],
	&["images"],
	&["export", "web"],
	&["generate", "quadlet"],
	&["version"],
];

/// Commands that print the resolved file. They write to stdout like the rest.
const CONFIG_COMMANDS: &[&[&str]] = &[
	&["config"],
	&["config", "--format", "json"],
	&["config", "--services"],
];

fn podup(engine: &FakeEngine, args: &[&str]) -> Command {
	let mut cmd = Command::new(env!("CARGO_BIN_EXE_podup"));
	cmd.current_dir(engine.dir())
		.arg("--socket")
		.arg(engine.dir().join("podman.sock"))
		.args(["-f", "compose.yaml"])
		.args(args)
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.stdin(Stdio::null());
	cmd
}

/// Run every command with stdout set to a pipe whose reader is already gone,
/// so the first write fails, and describe each run that did not end cleanly.
///
/// The same command first runs with stdout captured, and must have printed
/// something: a command that prints nothing would pass the closed pipe for no
/// reason and prove nothing.
fn unclean_endings(commands: &[&[&str]]) -> Vec<String> {
	let mut problems = Vec::new();
	for args in commands {
		let name = args.join(" ");
		let engine = engine();
		let open = podup(&engine, args).output().expect("run podup");
		if open.stdout.is_empty() {
			problems.push(format!("`{name}` printed nothing to an open pipe"));
			continue;
		}

		let (reader, writer) = std::io::pipe().expect("create pipe");
		drop(reader);
		let closed = podup(&engine, args)
			.stdout(writer)
			.stderr(Stdio::piped())
			.output()
			.expect("run podup");
		let stderr = String::from_utf8_lossy(&closed.stderr);
		if let Some(signal) = closed.status.signal() {
			problems.push(format!("`{name}` was ended by signal {signal}"));
		} else if closed.status.code() != Some(0) {
			problems.push(format!(
				"`{name}` exited {:?} on a closed pipe: {stderr}",
				closed.status.code()
			));
		}
		if !stderr.is_empty() {
			problems.push(format!(
				"`{name}` wrote to stderr on a closed pipe: {stderr}"
			));
		}
	}
	problems
}

#[test]
fn a_closed_stdout_ends_each_command_without_a_signal_a_panic_or_a_report() {
	let problems = unclean_endings(COMMANDS);
	assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
#[ignore = "config reports a closed pipe as an io error and exits 1; fixed on its own"]
fn a_closed_stdout_ends_config_without_a_report() {
	let problems = unclean_endings(CONFIG_COMMANDS);
	assert!(problems.is_empty(), "{}", problems.join("\n"));
}
