//! `run --remove-orphans` removes the containers of services the file no longer
//! declares before the one-off starts, and never before a bad service name is
//! refused. The engine is a fake Unix socket, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
name: demo
services:
  web:
    image: web-image
";

/// Lists one container of a service the file no longer declares.
fn orphan_listing(line: &str, _: &str) -> Option<Reply> {
	let is_list = line.starts_with("GET") && line.contains("/containers/json");
	is_list.then(|| Reply {
		status: 200,
		body: format!(
			r#"[{{"Id":"{id}","Names":["demo-gone-1"],"Image":"gone-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"demo","podup.service":"gone"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}]"#,
			id = "b".repeat(64),
		),
	})
}

fn run(args: &[&str]) -> (Option<i32>, String, Vec<String>) {
	let engine = FakeEngine::start(orphan_listing);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	let mut full = vec!["-f", "compose.yaml", "run", "-d"];
	full.extend_from_slice(args);
	let (code, output) = engine.run_with_status(None, &[], &full);
	let lines = engine
		.requests()
		.iter()
		.map(|r| {
			String::from_utf8_lossy(r)
				.lines()
				.next()
				.unwrap_or_default()
				.to_string()
		})
		.collect();
	(code, output, lines)
}

fn orphan_deleted(lines: &[String]) -> bool {
	lines
		.iter()
		.any(|l| l.starts_with("DELETE ") && l.contains("demo-gone-1"))
}

fn created(lines: &[String]) -> bool {
	lines
		.iter()
		.any(|l| l.starts_with("POST ") && l.contains("/containers/create"))
}

#[test]
fn the_orphan_is_removed_before_the_one_off_is_created() {
	let (code, output, lines) = run(&["--remove-orphans", "web", "true"]);
	assert_eq!(code, Some(0), "{output}");
	let removed = lines
		.iter()
		.position(|l| l.starts_with("DELETE ") && l.contains("demo-gone-1"))
		.unwrap_or_else(|| panic!("the orphan must be removed: {lines:?}"));
	let made = lines
		.iter()
		.position(|l| l.starts_with("POST ") && l.contains("/containers/create"))
		.unwrap_or_else(|| panic!("the one-off must be created: {lines:?}"));
	assert!(removed < made, "{lines:?}");
}

#[test]
fn without_the_flag_the_orphan_is_left_alone() {
	let (code, output, lines) = run(&["web", "true"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(!orphan_deleted(&lines), "{lines:?}");
	assert!(created(&lines), "{lines:?}");
}

#[test]
fn a_mistyped_service_is_refused_without_removing_anything() {
	let (code, output, lines) = run(&["--remove-orphans", "nope", "true"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("service 'nope' not found"), "{output}");
	assert!(!orphan_deleted(&lines), "{lines:?}");
	assert!(!created(&lines), "{lines:?}");
}
