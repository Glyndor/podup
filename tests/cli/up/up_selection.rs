//! `up` acts on what it was asked to start, and `config` reports what the file
//! declares. The engine is a fake Unix socket, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
  db:
    image: db-image
  debug:
    image: debug-image
    profiles: [debug]
";

fn accepts(_: &str, _: &str) -> Option<Reply> {
	None
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(accepts);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

#[test]
fn an_unknown_exit_code_from_is_refused_before_anything_starts() {
	let engine = engine();
	let output = engine.run(
		None,
		&[],
		&["-f", "compose.yaml", "up", "--exit-code-from", "typo"],
	);
	let sent = engine.sent();
	assert!(
		output.contains("typo"),
		"the error must name the service: {output}"
	);
	assert!(
		!sent.contains("POST") || !sent.contains("/containers/create"),
		"nothing may be created for an input error:\n{sent}"
	);
}

#[test]
fn config_profiles_lists_the_profiles_that_are_not_active() {
	let engine = engine();
	let output = engine.run(None, &[], &["-f", "compose.yaml", "config", "--profiles"]);
	assert!(
		output.lines().any(|l| l.trim() == "debug"),
		"an inactive profile is still declared: {output}"
	);
}

#[test]
fn config_hash_is_stable_across_runs() {
	let engine = FakeEngine::start(accepts);
	fs::write(
		engine.dir().join("compose.yaml"),
		"services:\n  web:\n    image: x\n    storage_opt:\n      a: '1'\n      b: '2'\n      c: '3'\n      d: '4'\n      e: '5'\n      f: '6'\n",
	)
	.unwrap();
	let first = engine.run(None, &[], &["-f", "compose.yaml", "config", "--hash", "*"]);
	for _ in 0..10 {
		let again = engine.run(None, &[], &["-f", "compose.yaml", "config", "--hash", "*"]);
		assert_eq!(first, again, "the hash of an unchanged file moved");
	}
}

#[test]
fn no_start_with_build_builds_the_image() {
	let engine = FakeEngine::start(accepts);
	fs::write(
		engine.dir().join("compose.yaml"),
		"services:\n  web:\n    build: .\n",
	)
	.unwrap();
	fs::write(engine.dir().join("Dockerfile"), "FROM scratch\n").unwrap();
	engine.run(
		None,
		&[],
		&["-f", "compose.yaml", "up", "--no-start", "--build"],
	);
	let sent = engine.sent();
	assert!(
		sent.contains("/build"),
		"`--build` must reach the build endpoint:\n{sent}"
	);
}
