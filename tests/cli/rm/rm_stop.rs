//! What `rm` asks the engine for: `-s` stops before it removes and resumes a
//! paused container first, `-f` and `-v` reach the removal request, and a
//! failed stop ends the command before anything is removed. The engine is a
//! fake Unix socket, so nothing here touches Podman.

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

fn container(state: &str) -> String {
	format!(
		r#"[{{"Id":"{id}","Names":["proj-web-1"],"Image":"web-image","ImageID":"","State":"{state}","Status":"","Labels":{{"podup.project":"proj","podup.service":"web"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}]"#,
		id = "a".repeat(64),
	)
}

fn is_list(line: &str) -> bool {
	line.starts_with("GET") && line.contains("/containers/json")
}

fn running(line: &str, _: &str) -> Option<Reply> {
	is_list(line).then(|| Reply {
		status: 200,
		body: container("running"),
	})
}

fn paused(line: &str, _: &str) -> Option<Reply> {
	is_list(line).then(|| Reply {
		status: 200,
		body: container("paused"),
	})
}

/// The engine refuses the stop with a server error.
fn stop_fails(line: &str, request: &str) -> Option<Reply> {
	if line.starts_with("POST") && line.contains("/stop") {
		return Some(Reply {
			status: 500,
			body: r#"{"cause":"boom","message":"cannot stop","response":500}"#.to_string(),
		});
	}
	running(line, request)
}

/// The engine refuses to remove a container that is still running, and says it
/// is running when it is asked.
fn remove_conflicts(line: &str, request: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("proj-web-1/json") {
		return Some(Reply {
			status: 200,
			body: r#"{"State":{"Status":"running"}}"#.to_string(),
		});
	}
	if line.starts_with("DELETE") && line.contains("/containers/") {
		return Some(Reply {
			status: 409,
			body: r#"{"cause":"running","message":"container is running","response":409}"#
				.to_string(),
		});
	}
	running(line, request)
}

fn engine(handler: fake_engine::Handler) -> FakeEngine {
	let engine = FakeEngine::start(handler);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

fn rm(engine: &FakeEngine, args: &[&str]) -> (Option<i32>, String) {
	let mut full = vec!["-f", "compose.yaml", "-p", "proj", "rm"];
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

fn position(lines: &[String], verb: &str, what: &str) -> Option<usize> {
	lines
		.iter()
		.position(|l| l.starts_with(verb) && l.contains(what))
}

#[test]
fn stop_stops_the_container_before_removing_it() {
	let engine = engine(running);
	let (code, output) = rm(&engine, &["-s", "web"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	let stopped = position(&lines, "POST", "proj-web-1/stop")
		.unwrap_or_else(|| panic!("the container must be stopped: {lines:?}"));
	let removed = position(&lines, "DELETE", "proj-web-1")
		.unwrap_or_else(|| panic!("the container must be removed: {lines:?}"));
	assert!(stopped < removed, "stop comes first: {lines:?}");
	assert!(
		lines[removed].contains("force=false"),
		"the removal itself is not forced: {lines:?}"
	);
}

#[test]
fn without_stop_nothing_is_stopped() {
	let engine = engine(running);
	let (code, output) = rm(&engine, &["web"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	assert!(
		position(&lines, "POST", "/stop").is_none(),
		"rm alone must not stop anything: {lines:?}"
	);
	assert!(
		position(&lines, "DELETE", "proj-web-1").is_some(),
		"{lines:?}"
	);
}

#[test]
fn stop_resumes_a_paused_container_before_stopping_it() {
	let engine = engine(paused);
	let (code, output) = rm(&engine, &["-s", "web"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	let resumed = position(&lines, "POST", "proj-web-1/unpause")
		.unwrap_or_else(|| panic!("a paused container must be resumed: {lines:?}"));
	let stopped = position(&lines, "POST", "proj-web-1/stop")
		.unwrap_or_else(|| panic!("the container must be stopped: {lines:?}"));
	let removed = position(&lines, "DELETE", "proj-web-1")
		.unwrap_or_else(|| panic!("the container must be removed: {lines:?}"));
	assert!(
		resumed < stopped && stopped < removed,
		"unpause, stop, remove, in that order: {lines:?}"
	);
}

#[test]
fn a_running_container_is_not_resumed() {
	let engine = engine(running);
	let (code, output) = rm(&engine, &["-s", "web"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(
		position(&lines(&engine), "POST", "/unpause").is_none(),
		"only a paused container is resumed: {:?}",
		lines(&engine)
	);
}

#[test]
fn a_failed_stop_ends_the_command_before_anything_is_removed() {
	let engine = engine(stop_fails);
	let (code, output) = rm(&engine, &["-s", "web"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("cannot stop"), "{output}");
	let lines = lines(&engine);
	assert!(
		position(&lines, "POST", "proj-web-1/stop").is_some(),
		"the stop was attempted: {lines:?}"
	);
	assert!(
		position(&lines, "DELETE", "/containers/").is_none(),
		"nothing may be removed after a failed stop: {lines:?}"
	);
}

#[test]
fn a_running_container_is_skipped_without_force_and_the_exit_is_clean() {
	let engine = engine(remove_conflicts);
	let (code, output) = rm(&engine, &["web"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(output.contains("pass -f to force removal"), "{output}");
}

#[test]
fn force_and_volumes_reach_the_removal_request() {
	let engine = engine(running);
	let (code, output) = rm(&engine, &["-f", "-v", "web"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = lines(&engine);
	let removed = position(&lines, "DELETE", "proj-web-1")
		.unwrap_or_else(|| panic!("the container must be removed: {lines:?}"));
	assert!(lines[removed].contains("force=true"), "{lines:?}");
	assert!(lines[removed].contains("volumes=true"), "{lines:?}");
	assert!(
		position(&lines, "POST", "/stop").is_none(),
		"-f removes without a separate stop: {lines:?}"
	);
}

#[test]
fn an_undeclared_service_is_refused_before_anything_is_stopped() {
	let engine = engine(running);
	let (code, output) = rm(&engine, &["-s", "nope"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("nope"), "{output}");
	let lines = lines(&engine);
	assert!(
		position(&lines, "POST", "/stop").is_none() && position(&lines, "DELETE", "/").is_none(),
		"nothing may change for a bad name: {lines:?}"
	);
}
