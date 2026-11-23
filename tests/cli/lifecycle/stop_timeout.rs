//! `-t/--timeout` on `up`, `down`, `stop` and `restart` is the grace period
//! Podman is told to wait before it kills a container, and it wins over the
//! `stop_grace_period` the file declares. The engine is a fake Unix socket
//! that lists one running container, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
";

const WITH_GRACE: &str = "\
services:
  web:
    image: web-image
    stop_grace_period: 20s
";

fn listing(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("/containers/json") {
		return Some(Reply {
			status: 200,
			body: format!(
				r#"[{{"Id":"{id}","Names":["proj-web-1"],"Image":"web-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"proj","podup.service":"web"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}]"#,
				id = "a".repeat(64),
			),
		});
	}
	// The container ends at once, so an attached `up` has a stream to follow
	// and then stops the project.
	if line.contains("/logs") {
		return Some(Reply {
			status: 200,
			body: "\u{1}\0\0\0\0\0\0\u{6}hello\n".into(),
		});
	}
	None
}

fn engine(compose: &str) -> FakeEngine {
	let engine = FakeEngine::start(listing);
	fs::write(engine.dir().join("compose.yaml"), compose).unwrap();
	engine
}

/// The `timeout=` of the request that stops (or restarts) the one container
/// of the project, or `None` when there is none. A teardown that works from a
/// snapshot names the container by its ID, so the name is not matched.
fn timeout_of(engine: &FakeEngine, action: &str) -> Option<String> {
	engine
		.sent()
		.lines()
		.find(|l| l.starts_with("POST") && l.contains(&format!("/{action}?timeout=")))
		.and_then(|l| l.split("timeout=").nth(1))
		.and_then(|rest| rest.split_whitespace().next())
		.map(str::to_string)
}

fn run(engine: &FakeEngine, args: &[&str]) -> (Option<i32>, String) {
	let mut full = vec!["-f", "compose.yaml", "-p", "proj"];
	full.extend_from_slice(args);
	engine.run_with_status(None, &[], &full)
}

#[test]
fn down_passes_its_timeout_to_the_stop() {
	let engine = engine(COMPOSE);
	let (code, output) = run(&engine, &["down", "-t", "7"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(timeout_of(&engine, "stop").as_deref(), Some("7"));
}

#[test]
fn down_without_a_timeout_uses_the_default_grace_period() {
	let engine = engine(COMPOSE);
	let (code, output) = run(&engine, &["down"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(timeout_of(&engine, "stop").as_deref(), Some("10"));
}

#[test]
fn down_without_a_timeout_uses_the_stop_grace_period_of_the_file() {
	let engine = engine(WITH_GRACE);
	let (code, output) = run(&engine, &["down"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(timeout_of(&engine, "stop").as_deref(), Some("20"));
}

#[test]
fn the_timeout_wins_over_the_stop_grace_period_of_the_file() {
	let engine = engine(WITH_GRACE);
	let (code, output) = run(&engine, &["down", "--timeout", "3"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(timeout_of(&engine, "stop").as_deref(), Some("3"));
}

#[test]
fn a_timeout_of_zero_is_sent_as_zero_and_not_as_the_default() {
	let engine = engine(COMPOSE);
	let (code, output) = run(&engine, &["down", "-t", "0"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(timeout_of(&engine, "stop").as_deref(), Some("0"));
}

#[test]
fn a_negative_timeout_is_refused_before_anything_is_stopped() {
	let engine = engine(COMPOSE);
	let (code, output) = run(&engine, &["down", "--timeout=-1"]);
	assert_eq!(code, Some(2), "{output}");
	assert!(output.contains("must be zero or greater"), "{output}");
	assert!(
		engine
			.sent()
			.lines()
			.all(|l| !l.starts_with("POST") && !l.starts_with("DELETE")),
		"nothing may change for a refused value:\n{}",
		engine.sent()
	);
}

#[test]
fn stop_passes_its_timeout_to_the_stop() {
	let engine = engine(COMPOSE);
	let (code, output) = run(&engine, &["stop", "-t", "4", "web"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(timeout_of(&engine, "stop").as_deref(), Some("4"));
}

#[test]
fn restart_passes_its_timeout_to_the_restart() {
	let engine = engine(COMPOSE);
	let (code, output) = run(&engine, &["restart", "-t", "5", "web"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(timeout_of(&engine, "restart").as_deref(), Some("5"));
}

#[test]
fn an_attached_up_passes_its_timeout_to_the_stop_that_ends_it() {
	let engine = engine(COMPOSE);
	let (code, output) = run(&engine, &["up", "-t", "6"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(
		timeout_of(&engine, "stop").as_deref(),
		Some("6"),
		"{}",
		engine.sent()
	);
}

/// With no compose file the project is found by its label, and the timeout
/// still reaches the stop.
#[test]
fn down_by_label_passes_its_timeout_to_the_stop() {
	let engine = FakeEngine::start(listing);
	let (code, output) = engine.run_with_status(None, &[], &["-p", "proj", "down", "-t", "9"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(timeout_of(&engine, "stop").as_deref(), Some("9"));
}
