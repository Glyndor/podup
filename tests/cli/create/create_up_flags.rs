//! `create` takes the flags of `up` that decide what gets created: `--scale`,
//! `--remove-orphans`, `--no-build` and `--quiet-pull`. The engine is a fake
//! Unix socket, so nothing here touches Podman.

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
  app:
    image: app-image
    build: .
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

/// No image is in local storage, so each one is built or pulled.
fn no_images(line: &str, _: &str) -> Option<Reply> {
	let is_inspect = line.starts_with("GET") && line.contains("/images/") && line.contains("/json");
	is_inspect.then(|| Reply {
		status: 404,
		body: r#"{"message":"no such image"}"#.to_string(),
	})
}

fn nothing_special(_: &str, _: &str) -> Option<Reply> {
	None
}

fn engine(handler: fake_engine::Handler) -> FakeEngine {
	let engine = FakeEngine::start(handler);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	fs::write(engine.dir().join("Dockerfile"), "FROM scratch\n").unwrap();
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

fn create(handler: fake_engine::Handler, args: &[&str]) -> (Option<i32>, String, Vec<String>) {
	let engine = engine(handler);
	let mut full = vec!["-f", "compose.yaml", "create"];
	full.extend_from_slice(args);
	let (code, output) = engine.run_with_status(None, &[], &full);
	(code, output, request_lines(&engine))
}

fn created(lines: &[String]) -> usize {
	lines
		.iter()
		.filter(|l| l.starts_with("POST ") && l.contains("/containers/create"))
		.count()
}

fn builds(lines: &[String]) -> usize {
	lines
		.iter()
		.filter(|l| l.starts_with("POST ") && l.contains("/build"))
		.count()
}

fn deleted(lines: &[String]) -> Vec<&String> {
	lines
		.iter()
		.filter(|l| l.starts_with("DELETE ") && l.contains("/containers/"))
		.collect()
}

#[test]
fn scale_sets_how_many_replicas_are_created() {
	let (code, output, lines) = create(nothing_special, &["--no-build", "web"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(created(&lines), 1, "{lines:?}");

	let (code, output, lines) = create(nothing_special, &["--no-build", "--scale", "web=3", "web"]);
	assert_eq!(code, Some(0), "{output}");
	assert_eq!(created(&lines), 3, "{lines:?}");
}

#[test]
fn scale_of_an_undeclared_service_is_refused_before_anything_changes() {
	let (code, output, lines) = create(nothing_special, &["--scale", "nope=2"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("service 'nope' not found"), "{output}");
	assert!(
		lines
			.iter()
			.all(|l| !l.starts_with("POST") || l.contains("_ping")),
		"{lines:?}"
	);
}

#[test]
fn remove_orphans_deletes_containers_of_services_no_longer_in_the_file() {
	let (code, output, lines) = create(orphan_listing, &["--no-build", "web"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(deleted(&lines).is_empty(), "{lines:?}");

	let (code, output, lines) = create(orphan_listing, &["--no-build", "--remove-orphans", "web"]);
	assert_eq!(code, Some(0), "{output}");
	let removed = deleted(&lines);
	assert!(
		removed.iter().any(|l| l.contains("demo-gone-1")),
		"the orphan must be removed: {lines:?}"
	);
	assert!(
		removed.iter().all(|l| !l.contains("demo-web")),
		"a declared service is not an orphan: {lines:?}"
	);
}

#[test]
fn no_build_skips_the_build_of_a_service_that_has_one() {
	let (_, output, lines) = create(no_images, &["app"]);
	assert!(
		builds(&lines) > 0,
		"control: `create` builds: {output}\n{lines:?}"
	);

	let (_, output, lines) = create(no_images, &["--no-build", "app"]);
	assert_eq!(builds(&lines), 0, "{output}\n{lines:?}");
}

#[test]
fn no_build_and_build_cannot_be_combined() {
	let (code, output, lines) = create(nothing_special, &["--build", "--no-build"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("cannot be used with"), "{output}");
	assert_eq!(created(&lines), 0, "{lines:?}");
}

#[test]
fn quiet_pull_hides_the_pull_progress_but_still_pulls() {
	let (code, output, lines) = create(no_images, &["web"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(output.contains("Image web-image"), "control: {output}");

	let (code, output, lines_quiet) = create(no_images, &["--quiet-pull", "web"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(!output.contains("Image web-image"), "{output}");
	let pulls = |l: &[String]| l.iter().filter(|r| r.contains("/images/pull")).count();
	assert_eq!(pulls(&lines_quiet), pulls(&lines), "{lines_quiet:?}");
	assert_eq!(created(&lines_quiet), 1, "{lines_quiet:?}");
}
