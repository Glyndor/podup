//! `down` with service names removes only those services' containers and leaves
//! the networks, the pod and the volumes the other services share. The engine is
//! a fake Unix socket that lists a `web` and a `db` container, so nothing here
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
    networks: [front]
  db:
    image: db-image
    build: .
    networks: [front]
networks:
  front: {}
volumes:
  data: {}
";

fn listing(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("/containers/json") {
		return Some(Reply {
			status: 200,
			body: r#"[
				{"Names":["proj-web-1"],"Labels":{"podup.service":"web"}},
				{"Names":["proj-db-1"],"Labels":{"podup.service":"db"}}
			]"#
			.to_string(),
		});
	}
	None
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(listing);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	fs::write(engine.dir().join("Dockerfile"), "FROM scratch\n").unwrap();
	engine
}

fn down(engine: &FakeEngine, args: &[&str]) -> (Option<i32>, String) {
	let mut full = vec!["-f", "compose.yaml", "-p", "proj", "down"];
	full.extend_from_slice(args);
	engine.run_with_status(None, &[], &full)
}

fn request_lines(engine: &FakeEngine) -> Vec<String> {
	engine
		.sent()
		.lines()
		.filter(|l| l.starts_with("GET ") || l.starts_with("POST ") || l.starts_with("DELETE "))
		.map(str::to_string)
		.collect()
}

#[test]
fn a_named_service_is_removed_and_the_others_are_left_running() {
	let engine = engine();
	let (code, output) = down(&engine, &["web"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = request_lines(&engine);
	assert!(
		lines
			.iter()
			.any(|l| l.starts_with("DELETE") && l.contains("proj-web-1")),
		"web must be removed: {lines:?}"
	);
	assert!(
		lines.iter().all(|l| !l.contains("proj-db-1")),
		"db must not be touched: {lines:?}"
	);
}

#[test]
fn naming_services_leaves_the_shared_networks_alone() {
	let engine = engine();
	let (code, output) = down(&engine, &["web"]);
	assert_eq!(code, Some(0), "{output}");
	let lines = request_lines(&engine);
	let touched: Vec<&String> = lines.iter().filter(|l| l.contains("/networks")).collect();
	assert!(
		touched.is_empty(),
		"a partial teardown must not reach the networks: {touched:?}"
	);
}

#[test]
fn without_names_the_whole_project_is_removed_with_its_networks() {
	let engine = engine();
	let (code, output) = down(&engine, &[]);
	assert_eq!(code, Some(0), "{output}");
	let lines = request_lines(&engine);
	for name in ["proj-web-1", "proj-db-1"] {
		assert!(
			lines
				.iter()
				.any(|l| l.starts_with("DELETE") && l.contains(name)),
			"{name} must be removed: {lines:?}"
		);
	}
	assert!(
		lines.iter().any(|l| l.contains("/networks")),
		"the project networks are part of a whole teardown: {lines:?}"
	);
}

#[test]
fn an_undeclared_service_is_refused_before_anything_is_removed() {
	let engine = engine();
	let (code, output) = down(&engine, &["web", "nope"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("service 'nope' not found"), "{output}");
	let lines = request_lines(&engine);
	assert!(
		lines
			.iter()
			.all(|l| !l.starts_with("DELETE") && !(l.starts_with("POST") && !l.contains("_ping"))),
		"nothing may change before the name is refused: {lines:?}"
	);
}

#[test]
fn volumes_cannot_be_removed_along_with_a_service_selection() {
	let engine = engine();
	let (code, output) = down(&engine, &["-v", "web"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	let lines = request_lines(&engine);
	assert!(
		lines.iter().all(|l| !l.contains("/volumes")),
		"no volume may be touched: {lines:?}"
	);
}

#[test]
fn rmi_with_names_removes_only_the_images_of_those_services() {
	let engine = engine();
	let (code, output) = down(&engine, &["--rmi", "all", "web"]);
	assert_eq!(code, Some(0), "{output}");
	let images: Vec<String> = request_lines(&engine)
		.into_iter()
		.filter(|l| l.starts_with("DELETE") && l.contains("/images/"))
		.collect();
	assert_eq!(images.len(), 1, "{images:?}");
	assert!(images[0].contains("web-image"), "{images:?}");
}
