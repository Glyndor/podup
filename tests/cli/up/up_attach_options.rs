//! Which services an attached `up` follows, and how the lines it prints look.
//! The engine is a fake Unix socket that answers a log request with one framed
//! line, so everything here runs without Podman.

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
";

/// One stdout frame as the libpod log endpoint sends it: channel byte, three
/// zero bytes, a big-endian length, then the payload.
fn logs_and_exits(line: &str, _request: &str) -> Option<Reply> {
	if line.contains("/logs") {
		return Some(Reply {
			status: 200,
			body: "\u{1}\0\0\0\0\0\0\u{6}hello\n".into(),
		});
	}
	// The database exits with a failure and the web service cleanly.
	if line.contains("proj-db-1/wait") {
		return Some(Reply {
			status: 200,
			body: "3".into(),
		});
	}
	if line.contains("/wait") {
		return Some(Reply {
			status: 200,
			body: "0".into(),
		});
	}
	None
}

const WITH_DEPENDENCY: &str = "\
services:
  web:
    image: web-image
    depends_on: [db]
  db:
    image: db-image
";

fn engine_with_dependency() -> FakeEngine {
	let engine = FakeEngine::start(logs_and_exits);
	fs::write(engine.dir().join("compose.yaml"), WITH_DEPENDENCY).unwrap();
	engine
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(logs_and_exits);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

fn up(engine: &FakeEngine, extra: &[&str]) -> (Option<i32>, String) {
	let mut args = vec!["-p", "proj", "-f", "compose.yaml", "up"];
	args.extend_from_slice(extra);
	engine.run_with_status(None, &[], &args)
}

fn followed(engine: &FakeEngine, container: &str) -> bool {
	engine
		.sent()
		.lines()
		.any(|l| l.contains(&format!("{container}/logs")))
}

#[test]
fn lines_carry_the_service_prefix_by_default() {
	let engine = engine();
	let (_, output) = up(&engine, &[]);
	assert!(output.contains("web-1 | hello"), "{output}");
}

#[test]
fn no_log_prefix_prints_the_bare_line() {
	let engine = engine();
	let (_, output) = up(&engine, &["--no-log-prefix"]);
	assert!(output.lines().any(|l| l == "hello"), "{output}");
	assert!(!output.contains(" | hello"), "{output}");
}

#[test]
fn attach_follows_only_the_named_service() {
	let engine = engine();
	up(&engine, &["--attach", "web"]);
	assert!(followed(&engine, "proj-web-1"));
	assert!(!followed(&engine, "proj-db-1"));
}

#[test]
fn no_attach_leaves_the_named_service_out() {
	let engine = engine();
	up(&engine, &["--no-attach", "web"]);
	assert!(!followed(&engine, "proj-web-1"));
	assert!(followed(&engine, "proj-db-1"));
}

#[test]
fn an_unknown_attach_name_is_refused_before_anything_starts() {
	for flag in ["--attach", "--no-attach"] {
		let engine = engine();
		let (status, output) = up(&engine, &[flag, "typo"]);
		assert_ne!(status, Some(0), "{output}");
		assert!(output.contains("typo"), "{output}");
		assert!(
			!engine.sent().contains("/containers/create"),
			"nothing may be created for an input error:\n{}",
			engine.sent()
		);
	}
}

#[test]
fn abort_on_container_failure_ignores_a_clean_exit_and_reports_the_failure() {
	let engine = engine();
	let (status, output) = up(&engine, &["--abort-on-container-failure"]);
	assert_eq!(status, Some(3), "{output}");
}

#[test]
fn a_named_service_is_followed_without_its_dependencies() {
	let engine = engine_with_dependency();
	up(&engine, &["web"]);
	assert!(followed(&engine, "proj-web-1"));
	assert!(!followed(&engine, "proj-db-1"));
	assert!(
		engine.sent().contains("/containers/proj-db-1/start"),
		"the dependency must still be started:\n{}",
		engine.sent()
	);
}

#[test]
fn attach_dependencies_follows_the_dependencies_too() {
	let engine = engine_with_dependency();
	up(&engine, &["web", "--attach-dependencies"]);
	assert!(followed(&engine, "proj-web-1"));
	assert!(followed(&engine, "proj-db-1"));
}

#[test]
fn with_no_service_named_every_service_is_followed() {
	let engine = engine_with_dependency();
	up(&engine, &[]);
	assert!(followed(&engine, "proj-web-1"));
	assert!(followed(&engine, "proj-db-1"));
}

#[test]
fn no_attach_still_wins_over_attach_dependencies() {
	let engine = engine_with_dependency();
	up(
		&engine,
		&["web", "--attach-dependencies", "--no-attach", "db"],
	);
	assert!(followed(&engine, "proj-web-1"));
	assert!(!followed(&engine, "proj-db-1"));
}
