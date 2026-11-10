//! The same service named twice in one scale request resolves to the last
//! count, and the replicas created and the replicas removed agree on it. The
//! engine is a fake Unix socket, so nothing here touches Podman.

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

fn container(name: &str) -> String {
	format!(
		r#"{{"Id":"{id}","Names":["{name}"],"Image":"web-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"demo","podup.service":"web"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}"#,
		id = "a".repeat(64),
	)
}

fn no_replicas(_: &str, _: &str) -> Option<Reply> {
	None
}

fn three_replicas(line: &str, _: &str) -> Option<Reply> {
	let is_list = line.starts_with("GET") && line.contains("/containers/json");
	is_list.then(|| Reply {
		status: 200,
		body: format!(
			"[{},{},{}]",
			container("demo-web-1"),
			container("demo-web-2"),
			container("demo-web-3")
		),
	})
}

fn engine(handler: fake_engine::Handler) -> FakeEngine {
	let engine = FakeEngine::start(handler);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

fn request_lines(engine: &FakeEngine) -> Vec<String> {
	engine
		.requests()
		.iter()
		.map(|r| {
			String::from_utf8_lossy(r)
				.lines()
				.next()
				.unwrap_or_default()
				.to_string()
		})
		.collect()
}

fn count(lines: &[String], prefix: &str) -> usize {
	lines.iter().filter(|l| l.starts_with(prefix)).count()
}

/// The containers a run deleted, by name.
fn deleted(lines: &[String]) -> Vec<String> {
	let mut names: Vec<String> = lines
		.iter()
		.filter_map(|l| l.strip_prefix("DELETE "))
		.filter_map(|l| l.split("/containers/").nth(1))
		.map(|rest| rest.split(['?', ' ']).next().unwrap_or("").to_string())
		.collect();
	names.sort();
	names
}

/// The containers a run started, by name.
fn started(lines: &[String]) -> Vec<String> {
	let mut names: Vec<String> = lines
		.iter()
		.filter(|l| l.starts_with("POST ") && l.contains("/start"))
		.filter_map(|l| l.split("/containers/").nth(1))
		.map(|rest| rest.split('/').next().unwrap_or("").to_string())
		.collect();
	names.sort();
	names
}

fn run(handler: fake_engine::Handler, args: &[&str]) -> Vec<String> {
	let engine = engine(handler);
	let mut full = vec!["-f", "compose.yaml"];
	full.extend_from_slice(args);
	let (code, output) = engine.run_with_status(None, &[], &full);
	assert_eq!(code, Some(0), "`{}`:\n{output}", args.join(" "));
	request_lines(&engine)
}

#[test]
fn scale_with_a_service_twice_creates_the_last_count() {
	let lines = run(no_replicas, &["scale", "web=1", "web=3"]);
	assert_eq!(count(&lines, "POST /v5.0.0/libpod/containers/create"), 3);
	assert_eq!(started(&lines), ["demo-web-1", "demo-web-2", "demo-web-3"]);
	assert!(deleted(&lines).is_empty(), "{lines:?}");
}

#[test]
fn scale_with_a_service_twice_removes_down_to_the_last_count() {
	let lines = run(three_replicas, &["scale", "web=3", "web=1"]);
	assert_eq!(deleted(&lines), ["demo-web-2", "demo-web-3"]);
	assert_eq!(count(&lines, "POST /v5.0.0/libpod/containers/create"), 0);
}

#[test]
fn scale_with_a_service_twice_keeps_every_replica_when_the_last_count_is_three() {
	let lines = run(three_replicas, &["scale", "web=1", "web=3"]);
	assert!(deleted(&lines).is_empty(), "{lines:?}");
	assert_eq!(count(&lines, "POST /v5.0.0/libpod/containers/create"), 0);
}

#[test]
fn a_repeated_up_scale_creates_the_last_count() {
	let lines = run(
		no_replicas,
		&["up", "-d", "--scale", "web=1", "--scale", "web=3"],
	);
	assert_eq!(count(&lines, "POST /v5.0.0/libpod/containers/create"), 3);
	assert_eq!(started(&lines), ["demo-web-1", "demo-web-2", "demo-web-3"]);
	assert!(deleted(&lines).is_empty(), "{lines:?}");
}

#[test]
fn a_repeated_up_scale_removes_down_to_the_last_count() {
	let lines = run(
		three_replicas,
		&["up", "-d", "--scale", "web=3", "--scale", "web=1"],
	);
	let removed = deleted(&lines);
	assert!(
		!removed.contains(&"demo-web-1".to_string()),
		"the replica inside the last count was removed: {removed:?}"
	);
	assert!(
		removed.contains(&"demo-web-2".to_string()) && removed.contains(&"demo-web-3".to_string()),
		"the replicas above the last count were kept: {removed:?}"
	);
}
