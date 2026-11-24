//! `pull --include-deps` also pulls what the named services depend on, and
//! `--ignore-pull-failures` turns a failed pull into a warning. The engine is
//! a fake Unix socket, so nothing here touches Podman or a registry.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
    depends_on: [api]
  api:
    image: api-image
    depends_on: [db]
  db:
    image: db-image
  cache:
    image: cache-image
";

fn accepts(_: &str, _: &str) -> Option<Reply> {
	None
}

/// The engine has no image `db-image` and cannot fetch it.
fn db_image_is_unreachable(line: &str, _: &str) -> Option<Reply> {
	if line.contains("images/pull") && line.contains("db-image") {
		return Some(Reply {
			status: 500,
			body: r#"{"cause":"unreachable","message":"db-image: registry unreachable","response":500}"#
				.to_string(),
		});
	}
	None
}

fn engine(handler: fake_engine::Handler) -> FakeEngine {
	let engine = FakeEngine::start(handler);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

fn pull(engine: &FakeEngine, args: &[&str]) -> (Option<i32>, String) {
	let mut full = vec!["-f", "compose.yaml", "pull", "--policy", "always"];
	full.extend_from_slice(args);
	engine.run_with_status(None, &[], &full)
}

/// The images the engine was asked to pull, in request order.
fn pulled(engine: &FakeEngine) -> Vec<String> {
	let mut images: Vec<String> = engine
		.requests()
		.iter()
		.filter_map(|r| String::from_utf8_lossy(r).lines().next().map(String::from))
		.filter(|l| l.contains("images/pull"))
		.filter_map(|l| {
			["web-image", "api-image", "db-image", "cache-image"]
				.into_iter()
				.find(|image| l.contains(image))
				.map(str::to_string)
		})
		.collect();
	images.sort();
	images
}

#[test]
fn a_named_service_is_pulled_alone_without_the_flag() {
	let engine = engine(accepts);
	let (code, output) = pull(&engine, &["web"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(pulled(&engine), ["web-image"]);
}

#[test]
fn include_deps_pulls_the_dependencies_transitively_and_nothing_else() {
	let engine = engine(accepts);
	let (code, output) = pull(&engine, &["--include-deps", "web"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(
		pulled(&engine),
		["api-image", "db-image", "web-image"],
		"the dependencies of the dependencies come too, an unrelated service does not"
	);
}

#[test]
fn include_deps_with_no_service_named_pulls_everything_once() {
	let engine = engine(accepts);
	let (code, output) = pull(&engine, &["--include-deps"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(
		pulled(&engine),
		["api-image", "cache-image", "db-image", "web-image"]
	);
}

#[test]
fn a_failed_pull_of_a_dependency_fails_the_command() {
	let engine = engine(db_image_is_unreachable);
	let (code, output) = pull(&engine, &["--include-deps", "web"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("db-image"), "{output}");
}

#[test]
fn without_include_deps_a_broken_dependency_is_never_asked_for() {
	let engine = engine(db_image_is_unreachable);
	let (code, output) = pull(&engine, &["web"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(!pulled(&engine).contains(&"db-image".to_string()));
}

#[test]
fn ignore_pull_failures_turns_the_failure_into_a_warning_and_pulls_the_rest() {
	let engine = engine(db_image_is_unreachable);
	let (code, output) = pull(&engine, &["--ignore-pull-failures"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(
		output.contains("db-image") && output.contains("ignored"),
		"the warning names the image and says it was ignored: {output}"
	);
	let images = pulled(&engine);
	for image in ["web-image", "api-image", "cache-image"] {
		assert!(
			images.contains(&image.to_string()),
			"{image} must still be pulled: {images:?}"
		);
	}
}

#[test]
fn the_same_failure_without_the_flag_stops_the_command() {
	let engine = engine(db_image_is_unreachable);
	let (code, output) = pull(&engine, &[]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("db-image"), "{output}");
}
