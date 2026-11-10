//! `pull --ignore-buildable` leaves out every service that declares a `build:`
//! section, even when it also names an `image:`, and still pulls the rest. The
//! engine is a fake Unix socket, so nothing here touches Podman.

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

fn accepts(_: &str, _: &str) -> Option<Reply> {
	None
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(accepts);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	fs::write(engine.dir().join("Dockerfile"), "FROM scratch\n").unwrap();
	engine
}

/// The request lines of every image pull the engine received.
fn pulls(engine: &FakeEngine) -> Vec<String> {
	engine
		.requests()
		.iter()
		.filter_map(|r| String::from_utf8_lossy(r).lines().next().map(String::from))
		.filter(|l| l.contains("images/pull"))
		.collect()
}

#[test]
fn pull_asks_for_a_buildable_service_image_without_the_flag() {
	let engine = engine();
	engine.run_with_status(
		None,
		&[],
		&["-f", "compose.yaml", "pull", "--policy", "always"],
	);
	let lines = pulls(&engine);
	assert!(lines.iter().any(|l| l.contains("web-image")), "{lines:?}");
	assert!(lines.iter().any(|l| l.contains("db-image")), "{lines:?}");
}

#[test]
fn pull_ignore_buildable_skips_services_with_a_build_section() {
	let engine = engine();
	engine.run_with_status(
		None,
		&[],
		&[
			"-f",
			"compose.yaml",
			"pull",
			"--policy",
			"always",
			"--ignore-buildable",
		],
	);
	let lines = pulls(&engine);
	assert!(
		lines.iter().all(|l| !l.contains("web-image")),
		"a buildable service was pulled: {lines:?}"
	);
	assert!(
		lines.iter().any(|l| l.contains("db-image")),
		"the image-only service was not pulled: {lines:?}"
	);
}

#[test]
fn pull_ignore_buildable_skips_a_buildable_service_named_on_the_command_line() {
	let engine = engine();
	engine.run_with_status(
		None,
		&[],
		&[
			"-f",
			"compose.yaml",
			"pull",
			"--policy",
			"always",
			"--ignore-buildable",
			"web",
		],
	);
	let lines = pulls(&engine);
	assert!(
		lines.is_empty(),
		"a buildable service was pulled: {lines:?}"
	);
}
