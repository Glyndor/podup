//! `up --watch` reports what it watches and what it syncs at the default log
//! level, as `watch` does, and `RUST_LOG` can still turn that off. The engine
//! is a fake Unix socket, so nothing here touches Podman.

#![cfg(all(unix, feature = "watch"))]

use std::fs;
use std::time::{Duration, Instant};

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;
#[path = "../../hostile/support/host_files.rs"]
mod host_files;

use fake_engine::{FakeEngine, Reply};
use host_files::{accepts, Running, BUDGET};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
    develop:
      watch:
        - action: sync
          path: ./src
          target: /dst
          initial_sync: true
";

/// A container of a service the file does not declare, which `--remove-orphans`
/// removes and logs at `info`; everything else is answered as usual.
fn with_an_orphan(line: &str, request: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("/containers/json") {
		return Some(Reply {
			status: 200,
			body: format!(
				r#"[{{"Id":"{id}","Names":["hostile-gone-1"],"Image":"gone-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"hostile","podup.service":"gone"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}]"#,
				id = "b".repeat(64),
			),
		});
	}
	accepts(line, request)
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(with_an_orphan);
	fs::create_dir(engine.dir().join("src")).unwrap();
	fs::write(engine.dir().join("src/file.txt"), "content").unwrap();
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

/// `podup up --watch`, with `RUST_LOG` as given (`None` leaves it unset, which
/// is the default level).
fn start(engine: &FakeEngine, rust_log: Option<&str>, extra: &[&str]) -> Running {
	let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_podup"));
	command
		.current_dir(engine.dir())
		.args([
			"--socket",
			engine.dir().join("podman.sock").to_str().unwrap(),
		])
		.args(["-f", "compose.yaml", "-p", "hostile", "up", "--watch"])
		.args(extra)
		.env("HOME", engine.dir())
		.env("XDG_RUNTIME_DIR", engine.dir())
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG");
	if let Some(level) = rust_log {
		command.env("RUST_LOG", level);
	}
	Running::command(command)
}

/// Wait until the initial sync has reached the engine: the one upload of the
/// archive is the proof that the watch ran and had its say.
fn wait_for_the_upload(engine: &FakeEngine) {
	let start = Instant::now();
	while !engine.requests().iter().any(|r| r.starts_with(b"PUT ")) {
		assert!(start.elapsed() < BUDGET, "no upload:\n{}", engine.sent());
		std::thread::sleep(Duration::from_millis(10));
	}
}

#[test]
fn the_default_level_shows_the_rule_count_and_the_sync() {
	let engine = engine();
	let mut child = start(&engine, None, &[]);
	child.until("watching 1 rule(s)");
	child.until("synced ");
}

#[test]
fn rust_log_warn_keeps_both_lines_out() {
	let engine = engine();
	let child = start(&engine, Some("warn"), &[]);
	wait_for_the_upload(&engine);
	let text = child.text();
	assert!(!text.contains("watching"), "{text}");
	assert!(!text.contains("synced"), "{text}");
}

#[test]
fn a_plain_up_does_not_raise_the_level() {
	let engine = engine();
	let out = std::process::Command::new(env!("CARGO_BIN_EXE_podup"))
		.current_dir(engine.dir())
		.args([
			"--socket",
			engine.dir().join("podman.sock").to_str().unwrap(),
		])
		.args(["-f", "compose.yaml", "-p", "hostile", "up", "-d"])
		.env("HOME", engine.dir())
		.env("XDG_RUNTIME_DIR", engine.dir())
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.stdin(std::process::Stdio::null())
		.output()
		.expect("run podup");
	let text = format!(
		"{}{}",
		String::from_utf8_lossy(&out.stdout),
		String::from_utf8_lossy(&out.stderr)
	);
	assert!(out.status.success(), "{text}");
	assert!(!text.contains("watching"), "{text}");
}

/// The raised level belongs to the watch loop alone: the rest of `up --watch`
/// stays at the default, so what the start-up phases log at `info` is not shown.
#[test]
fn only_the_watch_lines_are_raised_not_the_rest_of_up() {
	let engine = engine();
	let mut child = start(&engine, None, &["--remove-orphans"]);
	child.until("watching 1 rule(s)");
	child.until("synced ");
	wait_for_the_upload(&engine);
	assert!(
		engine
			.sent()
			.lines()
			.any(|l| l.starts_with("DELETE") && l.contains("hostile-gone-1")),
		"the orphan was removed, which is logged at info:\n{}",
		engine.sent()
	);
	let text = child.text();
	assert!(
		!text.contains("removing orphan container"),
		"another phase logged at info: {text}"
	);
}

/// The control for the test above: the line is there when the level is raised
/// for everything.
#[test]
fn the_orphan_line_exists_at_info() {
	let engine = engine();
	let mut child = start(&engine, Some("info"), &["--remove-orphans"]);
	child.until("removing orphan container hostile-gone-1");
}
