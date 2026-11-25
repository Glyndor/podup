//! `-t -1` is the grace period Podman reads as "wait for the container to stop
//! on its own": it reaches the engine as the largest value the engine takes, on
//! `up`, `down`, `stop` and `restart`, and anything below -1 is refused before a
//! request is sent. The engine is a fake Unix socket, so nothing here touches
//! Podman.

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
	if line.contains("/logs") {
		return Some(Reply {
			status: 200,
			body: "\u{1}\0\0\0\0\0\0\u{6}hello\n".into(),
		});
	}
	None
}

fn run(args: &[&str]) -> (FakeEngine, Option<i32>, String) {
	let engine = FakeEngine::start(listing);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	let mut full = vec!["-f", "compose.yaml", "-p", "proj"];
	full.extend_from_slice(args);
	let (code, output) = engine.run_with_status(None, &[], &full);
	(engine, code, output)
}

fn timeout_of(engine: &FakeEngine, action: &str) -> Option<String> {
	engine
		.sent()
		.lines()
		.find(|l| l.starts_with("POST") && l.contains(&format!("/{action}?timeout=")))
		.and_then(|l| l.split("timeout=").nth(1))
		.and_then(|rest| rest.split_whitespace().next())
		.map(str::to_string)
}

#[test]
fn minus_one_reaches_the_engine_as_its_largest_grace_period() {
	for (command, action) in [
		(vec!["down", "-t", "-1"], "stop"),
		(vec!["stop", "--timeout=-1", "web"], "stop"),
		(vec!["restart", "-t", "-1", "web"], "restart"),
		(vec!["up", "-t", "-1"], "stop"),
	] {
		let (engine, code, output) = run(&command);
		assert_eq!(code, Some(0), "{command:?}: {output}");
		assert_eq!(
			timeout_of(&engine, action),
			Some(i32::MAX.to_string()),
			"{command:?}: {}",
			engine.sent()
		);
	}
}

#[test]
fn anything_below_minus_one_is_refused_before_a_request_is_sent() {
	for command in [
		vec!["down", "-t", "-2"],
		vec!["stop", "--timeout=-2", "web"],
		vec!["restart", "-t", "-100", "web"],
		vec!["up", "-t", "-2"],
	] {
		let (engine, code, output) = run(&command);
		assert_eq!(code, Some(2), "{command:?}: {output}");
		assert!(
			output.contains("must be -1 (wait without limit)"),
			"{command:?}: {output}"
		);
		assert!(
			engine.requests().is_empty(),
			"{command:?}: {}",
			engine.sent()
		);
	}
}

#[test]
fn zero_and_positive_values_are_unchanged() {
	let (engine, code, output) = run(&["down", "-t", "0"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(timeout_of(&engine, "stop").as_deref(), Some("0"));
}
