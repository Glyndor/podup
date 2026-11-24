//! A remote build context can carry credentials in its user information or its
//! query. They must not reach a log line at any level: the line that says what
//! is being built shows the host and the path. The engine is a fake Unix
//! socket, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

// Each secret is made of pieces joined by characters that are valid in a
// password and in a token, so a cleaner that stops at one of them leaves the
// rest in view, and each piece is looked for on its own.
const PASSWORD: &str = "pwA1xx,pwB2xx(pwC3xx)pwD4xx'pwE5xx";
const TOKEN: &str = "tkF6yy,tkG7yy(tkH8yy)tkI9yy'tkJ0yy";

/// The pieces of both secrets, whole and one by one.
fn leaks(output: &str) -> Vec<&'static str> {
	let mut found = Vec::new();
	for secret in [PASSWORD, TOKEN] {
		if output.contains(secret) {
			found.push(secret);
		}
		for piece in secret.split([',', '(', ')', '\'']) {
			if output.contains(piece) {
				found.push(piece);
			}
		}
	}
	found
}

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
				leaks(&output).is_empty(),
				"{level} {command:?} showed {:?}:\n{output}",
				leaks(&output)
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
				leaks(&output).is_empty(),
				"{level} {command:?} repeated {:?}:\n{output}",
				leaks(&output)
			);
		}
	}
}

/// The build prints a step that fetches a URL with credentials in it, as
/// buildah echoes the instruction it runs.
fn echoing_a_step(line: &str, request: &str) -> Option<Reply> {
	if line.starts_with("POST") && line.contains("/build") {
		return Some(Reply {
			status: 200,
			body: format!(
				"{{\"stream\":\"STEP 1/2: ADD https://user:{PASSWORD}@host.example/f.tgz?token={TOKEN} /f\\n\"}}\n{{\"stream\":\"--> sha256:0123456789abcdef\\n\"}}\n"
			),
		});
	}
	no_images(line, request)
}

#[test]
fn a_build_step_that_echoes_a_url_does_not_repeat_its_credentials() {
	let engine = FakeEngine::start(echoing_a_step);
	fs::write(
		engine.dir().join("compose.yaml"),
		"services:\n  app:\n    image: app-image\n    build: .\n",
	)
	.unwrap();
	fs::write(engine.dir().join("Dockerfile"), "FROM scratch\n").unwrap();
	let (_, output) =
		engine.run_with_status(None, &[], &["-f", "compose.yaml", "-p", "proj", "build"]);
	assert!(
		output.contains("STEP 1/2: ADD https://host.example/f.tgz"),
		"{output}"
	);
	assert!(leaks(&output).is_empty(), "{:?}:\n{output}", leaks(&output));
}
