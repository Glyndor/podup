//! `run --env-from-file FILE` loads variables from a file into the one-off
//! container: above the service's `environment:`, below `-e`, repeatable, and
//! a file that is missing or malformed is an error that names it. The engine is
//! a fake Unix socket, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
    environment:
      FROM_SERVICE: service
      SHARED: service
";

fn accepts(_: &str, _: &str) -> Option<Reply> {
	None
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(accepts);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

/// The body of the container-create request the engine received.
fn create_body(engine: &FakeEngine) -> String {
	engine
		.requests()
		.iter()
		.map(|r| String::from_utf8_lossy(r).into_owned())
		.find(|r| r.starts_with("POST") && r.contains("containers/create"))
		.unwrap_or_default()
}

fn run_with(engine: &FakeEngine, extra: &[&str]) -> (Option<i32>, String) {
	let mut args = vec!["-f", "compose.yaml", "run", "-d", "--no-deps"];
	args.extend_from_slice(extra);
	args.push("web");
	engine.run_with_status(None, &[], &args)
}

#[test]
fn run_env_from_file_puts_the_file_variables_in_the_container() {
	let engine = engine();
	fs::write(engine.dir().join("a.env"), "FROM_FILE=file\nSHARED=file\n").unwrap();
	let (code, output) = run_with(&engine, &["--env-from-file", "a.env"]);
	let body = create_body(&engine);
	assert_eq!(code, Some(0), "{output}");
	assert!(body.contains("\"FROM_FILE\":\"file\""), "{body}");
	assert!(body.contains("\"FROM_SERVICE\":\"service\""), "{body}");
	assert!(
		body.contains("\"SHARED\":\"file\""),
		"the file must win over the service: {body}"
	);
}

#[test]
fn run_env_from_file_loses_to_the_command_line_and_later_files_win() {
	let engine = engine();
	fs::write(engine.dir().join("a.env"), "ONE=a\nTWO=a\nTHREE=a\n").unwrap();
	fs::write(engine.dir().join("b.env"), "TWO=b\nTHREE=b\n").unwrap();
	let (code, output) = run_with(
		&engine,
		&[
			"--env-from-file",
			"a.env",
			"--env-from-file",
			"b.env",
			"-e",
			"THREE=cli",
		],
	);
	let body = create_body(&engine);
	assert_eq!(code, Some(0), "{output}");
	assert!(body.contains("\"ONE\":\"a\""), "{body}");
	assert!(body.contains("\"TWO\":\"b\""), "{body}");
	assert!(body.contains("\"THREE\":\"cli\""), "{body}");
}

#[test]
fn run_env_from_file_names_a_missing_file() {
	let engine = engine();
	let (code, output) = run_with(&engine, &["--env-from-file", "absent.env"]);
	assert!(matches!(code, Some(c) if c != 0), "{code:?}: {output}");
	assert!(output.contains("absent.env"), "{output}");
	assert!(create_body(&engine).is_empty(), "container was created");
}

#[test]
fn run_env_from_file_names_a_malformed_file() {
	let engine = engine();
	fs::write(
		engine.dir().join("bad.env"),
		"OK=1\nBROKEN=\"never closed\n",
	)
	.unwrap();
	let (code, output) = run_with(&engine, &["--env-from-file", "bad.env"]);
	assert!(matches!(code, Some(c) if c != 0), "{code:?}: {output}");
	assert!(output.contains("bad.env"), "{output}");
	assert!(create_body(&engine).is_empty(), "container was created");
}
