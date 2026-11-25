//! `up --watch` starts the project and then keeps running to sync the files of
//! the `develop.watch` rules, where a plain `up -d` returns. The engine is a
//! fake Unix socket, so nothing here touches Podman.

#![cfg(all(unix, feature = "watch"))]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;
#[path = "../../hostile/support/host_files.rs"]
mod host_files;

use fake_engine::FakeEngine;
use host_files::{accepts, Running};

/// Start `podup up --watch` with the log level `watch` itself runs at: the
/// rule count is logged at info, which the default floor of `up` leaves out.
fn start_up_watch(engine: &FakeEngine) -> Running {
	let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_podup"));
	command
		.current_dir(engine.dir())
		.args([
			"--socket",
			engine.dir().join("podman.sock").to_str().unwrap(),
		])
		.args(["-f", "compose.yaml", "-p", "hostile", "up", "--watch"])
		.env("HOME", engine.dir())
		.env("XDG_RUNTIME_DIR", engine.dir())
		.env("NO_COLOR", "1")
		.env("RUST_LOG", "info");
	Running::command(command)
}

const COMPOSE: &str = "\
services:
  web:
    image: web-image
    develop:
      watch:
        - action: sync
          path: ./src
          target: /dst
";

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(accepts);
	fs::create_dir(engine.dir().join("src")).unwrap();
	fs::write(engine.dir().join("src/file.txt"), "content").unwrap();
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

fn created(engine: &FakeEngine) -> bool {
	engine
		.sent()
		.lines()
		.any(|l| l.starts_with("POST") && l.contains("/containers/create"))
}

#[test]
fn up_watch_creates_the_containers_and_then_watches_the_rules() {
	let engine = engine();
	let mut child = start_up_watch(&engine);
	child.until("watching 1 rule(s)");
	assert!(
		created(&engine),
		"the project is started before it is watched:\n{}",
		engine.sent()
	);
}

#[test]
fn up_without_watch_returns_and_never_watches() {
	let engine = engine();
	let mut child = Running::start(&engine, &["up", "-d"]);
	let status = child.wait();
	assert!(status.success(), "{}", child.text());
	assert!(created(&engine), "{}", engine.sent());
	assert!(
		!child.text().contains("watching"),
		"no watch without the flag: {}",
		child.text()
	);
}
