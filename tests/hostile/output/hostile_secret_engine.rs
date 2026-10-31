//! Secret values must stay out of what podup prints once it reaches an engine.
//!
//! `hostile_secret_output.rs` runs against a dead socket, which stops every
//! command at the first request. Here a fake engine answers `_ping` and then
//! accepts whatever podup sends, so `up` and `create` go on to stage the
//! `secrets:` of the compose file. The markers are values no honest output
//! contains: nothing podup prints, at the default level or at `RUST_LOG=debug`,
//! may carry the content of a secret, and at the default level the values of
//! the fields `config` shows by design (environment, env file, build argument,
//! a password inside a URL) stay out too.
//!
//! Two controls keep the scan honest: the engine records what it was sent and
//! a test requires the secret bytes to be among it (so the code that handles
//! them ran), and the engine can refuse the secret request, which is the path
//! where an error message could echo what it was given.

// The fake engine is a Unix domain socket server.
#![cfg(unix)]

use std::fs;

#[path = "../support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const INLINE: &str = "MARK-inline-content-7f3a";
const FILE: &str = "MARK-secret-file-bytes-91c2";
const FROM_ENV: &str = "MARK-env-sourced-secret-5d10";
/// Values of fields `config` shows by design; a command that only reports
/// names and failures has no reason to print them.
const ENV_VALUE: &str = "MARK-service-env-value-2b8e";
const ENV_FILE: &str = "MARK-env-file-value-c47d";
const BUILD_ARG: &str = "MARK-build-arg-value-08fa";
const URL_PASSWORD: &str = "MARK-url-password-e6b1";

const COMPOSE: &str = "\
services:
  web:
    image: registry.example/app:1
    environment:
      DB_PASSWORD: MARK-service-env-value-2b8e
    env_file: ./app.env
    build:
      context: https://user:MARK-url-password-e6b1@git.example/repo.git
      args:
        TOKEN: MARK-build-arg-value-08fa
    secrets: [inline, fromenv, fromfile]
secrets:
  inline:
    content: MARK-inline-content-7f3a
  fromenv:
    environment: SECRET_SRC
  fromfile:
    file: ./secret.txt
";

const COMMANDS: &[&[&str]] = &[&["up", "-d"], &["create"], &["up", "-d", "--build"]];

fn accepts(_: &str, _: &str) -> Option<Reply> {
	None
}

fn refuses_secrets(line: &str, _: &str) -> Option<Reply> {
	line.contains("/secrets/create").then(|| Reply {
		status: 500,
		body: "{\"cause\":\"refused\",\"message\":\"the engine refuses this\",\"response\":500}"
			.into(),
	})
}

fn engine(handler: fake_engine::Handler) -> FakeEngine {
	let fake = FakeEngine::start(handler);
	fs::write(fake.dir().join("compose.yaml"), COMPOSE).unwrap();
	fs::write(fake.dir().join("secret.txt"), FILE).unwrap();
	fs::write(fake.dir().join("app.env"), format!("FILE_VAR={ENV_FILE}\n")).unwrap();
	fake
}

fn run(fake: &FakeEngine, log: Option<&str>, args: &[&str]) -> String {
	let mut full = vec!["-f", "compose.yaml", "-p", "leak"];
	full.extend_from_slice(args);
	fake.run(log, &[("SECRET_SRC", FROM_ENV)], &full)
}

/// The markers `text` carries that its log level must not show. The debug
/// level is documented to possibly show environment values
/// (docs/security-model.md), so there only the sources of `secrets:` are held
/// to the rule.
fn found(text: &str, log: Option<&str>) -> Vec<&'static str> {
	let mut markers = vec![INLINE, FILE, FROM_ENV];
	if log.is_none() {
		markers.extend([ENV_VALUE, ENV_FILE, BUILD_ARG, URL_PASSWORD]);
	}
	markers.into_iter().filter(|m| text.contains(m)).collect()
}

#[test]
fn the_engine_receives_the_secrets_the_scan_is_looking_for() {
	// The control: if no command ever sends the secret bytes to the engine,
	// the checks below only prove that podup stopped early.
	let fake = engine(accepts);
	for args in COMMANDS {
		run(&fake, None, args);
	}
	let sent = fake.sent();
	assert!(
		sent.contains(INLINE) && sent.contains(FILE) && sent.contains(FROM_ENV),
		"the secret bytes did not all reach the fake engine; requests were:\n{}",
		sent.lines()
			.filter(|l| l.contains("HTTP/1.1"))
			.collect::<Vec<_>>()
			.join("\n")
	);
}

#[test]
fn secret_content_is_not_printed_when_the_engine_accepts() {
	for log in [None, Some("debug")] {
		let fake = engine(accepts);
		for args in COMMANDS {
			let text = run(&fake, log, args);
			assert!(
				found(&text, log).is_empty(),
				"podup {} (log {log:?}) printed {:?}:\n{text}",
				args.join(" "),
				found(&text, log)
			);
		}
	}
}

#[test]
fn secret_content_is_not_printed_when_the_engine_refuses_it() {
	for log in [None, Some("debug")] {
		let fake = engine(refuses_secrets);
		for args in COMMANDS {
			let text = run(&fake, log, args);
			assert!(
				found(&text, log).is_empty(),
				"podup {} (log {log:?}) printed {:?}:\n{text}",
				args.join(" "),
				found(&text, log)
			);
		}
	}
}
