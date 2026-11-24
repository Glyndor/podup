//! A remote build context can carry credentials in its user information or its
//! query. They must not reach a log line at any level: the line that says what
//! is being built shows the host and the path. The engine is a fake Unix
//! socket, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const PASSWORD: &str = "s3cr3tpw";
const TOKEN: &str = "t0k3nval";

fn compose() -> String {
	format!(
		"services:\n  app:\n    image: app-image\n    build:\n      context: \"https://user:{PASSWORD}@host.example/org/repo.git?token={TOKEN}#main\"\n"
	)
}

/// The image is not in local storage, so the build runs.
fn no_images(line: &str, _: &str) -> Option<Reply> {
	let is_inspect = line.starts_with("GET") && line.contains("/images/") && line.contains("/json");
	is_inspect.then(|| Reply {
		status: 404,
		body: r#"{"message":"no such image"}"#.to_string(),
	})
}

/// The daemon cannot clone the context and says so the way Podman does: with
/// the URL it was given, and with what git adds to it.
fn clone_fails(line: &str, request: &str) -> Option<Reply> {
	if line.starts_with("POST") && line.contains("/build") {
		return Some(Reply {
			status: 500,
			body: format!(
				r#"{{"cause":"exit status 128","message":"cloning \"https://user:{PASSWORD}@host.example/org/repo.git?token={TOKEN}#main\" to \"/var/tmp/b\":\nfatal: unable to access 'https://host.example/org/repo.git?token={TOKEN}/': could not resolve host","response":500}}"#
			),
		});
	}
	no_images(line, request)
}

fn build(level: &str, command: &[&str]) -> String {
	run(no_images, level, command)
}

fn run(handler: fake_engine::Handler, level: &str, command: &[&str]) -> String {
	let engine = FakeEngine::start(handler);
	fs::write(engine.dir().join("compose.yaml"), compose()).unwrap();
	let mut args = vec!["-f", "compose.yaml", "-p", "proj"];
	args.extend_from_slice(command);
	let (_, output) = engine.run_with_status(Some(level), &[], &args);
	output
}

#[test]
fn no_log_level_shows_the_password_or_the_token() {
	for level in ["error", "warn", "info", "debug", "trace"] {
		for command in [&["build"][..], &["up", "-d", "--build"]] {
			let output = build(level, command);
			assert!(
				!output.contains(PASSWORD),
				"{level} {command:?} showed the password:\n{output}"
			);
			assert!(
				!output.contains(TOKEN),
				"{level} {command:?} showed the token:\n{output}"
			);
		}
	}
}

#[test]
fn the_line_that_says_what_is_built_still_names_the_host_and_the_path() {
	let output = build("info", &["build"]);
	assert!(
		output.contains("building from remote context https://host.example/org/repo.git#main"),
		"{output}"
	);
}

#[test]
fn a_clone_that_fails_does_not_repeat_the_credentials_in_the_error() {
	for level in ["error", "info", "trace"] {
		for command in [&["build"][..], &["up", "-d", "--build"]] {
			let output = run(clone_fails, level, command);
			assert!(
				output.contains("host.example/org/repo.git"),
				"{level} {command:?} lost the host and the path:\n{output}"
			);
			assert!(
				!output.contains(PASSWORD) && !output.contains(TOKEN),
				"{level} {command:?} repeated a credential:\n{output}"
			);
		}
	}
}
