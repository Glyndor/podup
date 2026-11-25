//! `up --remove-orphans` removes the containers of services the file no longer
//! declares before it creates anything, and without the flag it only warns.
//! The engine is a fake Unix socket that lists one orphan, so nothing here
//! touches Podman.

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

fn orphan(name: &str, service: &str, project: &str) -> String {
	format!(
		r#"{{"Id":"{id}","Names":["{name}"],"Image":"gone-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"{project}","podup.service":"{service}"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}"#,
		id = "b".repeat(64),
	)
}

/// The project holds one container of a service the file does not declare.
fn one_orphan(line: &str, _: &str) -> Option<Reply> {
	(line.starts_with("GET") && line.contains("/containers/json")).then(|| Reply {
		status: 200,
		body: format!("[{}]", orphan("proj-gone-1", "gone", "proj")),
	})
}

/// The orphan cannot be removed: the engine answers with a server error.
fn orphan_cannot_be_removed(line: &str, request: &str) -> Option<Reply> {
	if line.starts_with("DELETE") && line.contains("proj-gone-1") {
		return Some(Reply {
			status: 500,
			body: r#"{"cause":"busy","message":"orphan is busy","response":500}"#.to_string(),
		});
	}
	one_orphan(line, request)
}

fn engine(handler: fake_engine::Handler) -> FakeEngine {
	let engine = FakeEngine::start(handler);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

fn up(engine: &FakeEngine, args: &[&str]) -> (Option<i32>, String) {
	let mut full = vec!["-f", "compose.yaml", "-p", "proj", "up", "-d"];
	full.extend_from_slice(args);
	engine.run_with_status(None, &[], &full)
}

fn lines(engine: &FakeEngine) -> Vec<String> {
	engine
		.sent()
		.lines()
		.filter(|l| l.starts_with("GET ") || l.starts_with("POST ") || l.starts_with("DELETE "))
		.map(str::to_string)
		.collect()
}

fn orphan_deleted(lines: &[String]) -> Option<usize> {
	lines
		.iter()
		.position(|l| l.starts_with("DELETE ") && l.contains("proj-gone-1"))
}

fn created(lines: &[String]) -> Option<usize> {
	lines
		.iter()
		.position(|l| l.starts_with("POST ") && l.contains("/containers/create"))
}

#[test]
fn the_orphan_is_removed_before_the_service_is_created() {
	let engine = engine(one_orphan);
	let (code, output) = up(&engine, &["--remove-orphans"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	let removed =
		orphan_deleted(&lines).unwrap_or_else(|| panic!("the orphan must be removed: {lines:?}"));
	let made = created(&lines).unwrap_or_else(|| panic!("web must be created: {lines:?}"));
	assert!(removed < made, "{lines:?}");
	assert!(
		lines[removed].contains("force=true"),
		"an orphan is removed even when it runs: {lines:?}"
	);
}

#[test]
fn without_the_flag_the_orphan_is_left_alone_and_named_in_a_warning() {
	let engine = engine(one_orphan);
	let (code, output) = up(&engine, &[]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	assert!(orphan_deleted(&lines).is_none(), "{lines:?}");
	assert!(created(&lines).is_some(), "{lines:?}");
	assert!(
		output.contains("orphan container") && output.contains("proj-gone-1"),
		"the warning names the orphan: {output}"
	);
	assert!(output.contains("--remove-orphans"), "{output}");
}

#[test]
fn removing_orphans_does_not_warn_about_them() {
	let engine = engine(one_orphan);
	let (code, output) = up(&engine, &["--remove-orphans"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(!output.contains("found orphan container"), "{output}");
}

#[test]
fn a_service_the_file_declares_is_never_taken_for_an_orphan() {
	fn declared(line: &str, _: &str) -> Option<Reply> {
		(line.starts_with("GET") && line.contains("/containers/json")).then(|| Reply {
			status: 200,
			body: format!("[{}]", orphan("proj-web-1", "web", "proj")),
		})
	}
	let engine = engine(declared);
	let (code, output) = up(&engine, &["--remove-orphans"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	assert!(
		lines.iter().all(|l| !l.starts_with("DELETE ")),
		"nothing may be removed: {lines:?}"
	);
	assert!(
		!output.contains("found orphan container"),
		"a declared service is not an orphan: {output}"
	);
}

#[test]
fn an_orphan_that_cannot_be_removed_fails_the_command_before_anything_is_created() {
	let engine = engine(orphan_cannot_be_removed);
	let (code, output) = up(&engine, &["--remove-orphans"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("orphan is busy"), "{output}");
	assert!(
		created(&lines(&engine)).is_none(),
		"nothing is created after a failed sweep: {:?}",
		lines(&engine)
	);
}

#[test]
fn a_mistyped_service_is_refused_without_removing_the_orphan() {
	let engine = engine(one_orphan);
	let (code, output) = up(&engine, &["--remove-orphans", "nope"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("service 'nope' not found"), "{output}");
	let lines = lines(&engine);
	assert!(orphan_deleted(&lines).is_none(), "{lines:?}");
	assert!(created(&lines).is_none(), "{lines:?}");
}
