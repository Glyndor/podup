//! A service name the file does not declare is a usage error for every command
//! that takes one: a non-zero exit, a message that names the service, and no
//! request to the engine beyond the version ping. The engine is a fake Unix
//! socket, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
    build: .
  db:
    image: db-image
";

const COMMANDS: &[&[&str]] = &[
	&["up", "nope"],
	&["up", "--no-start", "nope"],
	&["up", "--scale", "nope=2"],
	&["up", "--no-start", "--scale", "nope=2"],
	&["create", "nope"],
	&["scale", "nope=2"],
	&["stats", "--no-stream", "nope"],
	&["pull", "nope"],
	&["build", "nope"],
	&["push", "nope"],
];

fn accepts(_: &str, _: &str) -> Option<Reply> {
	None
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(accepts);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	fs::write(engine.dir().join("Dockerfile"), "FROM scratch\n").unwrap();
	engine
}

/// Every request the engine saw, as its request line.
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

#[test]
fn an_undeclared_service_is_refused_before_the_engine_is_asked_for_anything() {
	let mut failures = Vec::new();
	for args in COMMANDS {
		let engine = engine();
		let mut full = vec!["-f", "compose.yaml"];
		full.extend_from_slice(args);
		let (code, output) = engine.run_with_status(None, &[], &full);
		let name = args.join(" ");

		if !matches!(code, Some(c) if c != 0) {
			failures.push(format!("`{name}` exited {code:?}, not non-zero:\n{output}"));
		}
		if !output.contains("service 'nope' not found") {
			failures.push(format!(
				"`{name}` did not name the missing service:\n{output}"
			));
		}
		let lines = request_lines(&engine);
		let beyond_ping: Vec<&String> = lines.iter().filter(|l| !l.contains("_ping")).collect();
		if !beyond_ping.is_empty() {
			failures.push(format!(
				"`{name}` reached the engine before refusing the name: {beyond_ping:?}"
			));
		}
	}
	assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_declared_service_next_to_an_undeclared_one_is_still_refused() {
	for args in [
		&["up", "--no-start", "web", "nope"][..],
		&["create", "web", "nope"][..],
		&["pull", "web", "nope"][..],
		&["push", "web", "nope"][..],
	] {
		let engine = engine();
		let mut full = vec!["-f", "compose.yaml"];
		full.extend_from_slice(args);
		let (code, output) = engine.run_with_status(None, &[], &full);
		assert!(
			matches!(code, Some(c) if c != 0),
			"`{}` must exit non-zero, got {code:?}:\n{output}",
			args.join(" ")
		);
		let lines = request_lines(&engine);
		assert!(
			lines
				.iter()
				.all(|l| !l.starts_with("POST") || l.contains("_ping")),
			"`{}` changed state before refusing the unknown name: {lines:?}",
			args.join(" ")
		);
	}
}
