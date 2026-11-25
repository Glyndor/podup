//! The contexts a build takes besides its own: an image named with
//! credentials, or a URL that carries them. A message of the engine that
//! repeats only the password must not show it. The engine is a fake Unix
//! socket, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const PASSWORD: &str = "pwK4xx,pwL5xx(pwM6xx)pwN7xx";
const TOKEN: &str = "tkP8yy,tkQ9yy(tkR0yy)tkS1yy";

fn compose(context: &str) -> String {
	format!(
		"services:\n  app:\n    image: app-image\n    build:\n      context: ctx\n      additional_contexts:\n        extra: \"{context}\"\n"
	)
}

fn no_images(line: &str, _: &str) -> Option<Reply> {
	let is_inspect = line.starts_with("GET") && line.contains("/images/") && line.contains("/json");
	is_inspect.then(|| Reply {
		status: 404,
		body: r#"{"message":"no such image"}"#.to_string(),
	})
}

/// The engine refuses the context and repeats only the secrets.
fn refuses(line: &str, request: &str) -> Option<Reply> {
	if line.starts_with("POST") && line.contains("/build") {
		return Some(Reply {
			status: 500,
			body: format!(
				r#"{{"cause":"refused","message":"could not use the context: {PASSWORD} and {TOKEN}","response":500}}"#
			),
		});
	}
	no_images(line, request)
}

fn build(context: &str) -> String {
	let engine = FakeEngine::start(refuses);
	fs::create_dir(engine.dir().join("ctx")).unwrap();
	fs::write(engine.dir().join("ctx/Dockerfile"), "FROM scratch\n").unwrap();
	fs::write(engine.dir().join("compose.yaml"), compose(context)).unwrap();
	let (_, output) = engine.run_with_status(
		Some("error"),
		&[],
		&["-f", "compose.yaml", "-p", "proj", "build"],
	);
	output
}

fn pieces_shown(output: &str) -> usize {
	[PASSWORD, TOKEN]
		.iter()
		.flat_map(|secret| secret.split([',', '(', ')']))
		.filter(|piece| output.contains(piece))
		.count()
}

#[test]
fn an_image_named_as_a_context_with_credentials_is_not_repeated() {
	let output = build(&format!(
		"docker-image://{TOKEN}:{PASSWORD}@127.0.0.1:1/team/base:1"
	));
	assert!(
		output.contains("could not use the context"),
		"the engine error is missing"
	);
	assert!(
		pieces_shown(&output) == 0,
		"{} piece(s) of a secret came out",
		pieces_shown(&output)
	);
}

#[test]
fn a_url_as_a_context_with_a_token_is_not_repeated() {
	let output = build(&format!(
		"https://usrU3zz:{PASSWORD}@host.example/org/repo.git?token={TOKEN}"
	));
	assert!(
		output.contains("could not use the context"),
		"the engine error is missing"
	);
	assert!(
		pieces_shown(&output) == 0,
		"{} piece(s) of a secret came out",
		pieces_shown(&output)
	);
}
