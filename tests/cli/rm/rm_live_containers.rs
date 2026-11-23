//! `rm` without `-f` skips a container that is running or paused, with a
//! warning, and goes on to remove the ones that have stopped. Which state it is
//! in is asked of the container, not read from the status code of the refused
//! removal: Podman 6.2.0 answers 500 and other versions 409. The engine is a
//! fake Unix socket, so nothing here touches Podman.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const COMPOSE: &str = "\
services:
  web:
    image: web-image
";

fn listing() -> Reply {
	Reply {
		status: 200,
		body: format!(
			r#"[{{"Id":"{id}","Names":["proj-web-1"],"Image":"web-image","ImageID":"","State":"running","Status":"","Labels":{{"podup.project":"proj","podup.service":"web"}},"Created":"2026-08-01T00:58:45Z","StartedAt":1,"Ports":[]}}]"#,
			id = "a".repeat(64),
		),
	}
}

fn refused(status: u16) -> Reply {
	Reply {
		status,
		body: r#"{"cause":"container state improper","message":"cannot remove container: running or paused containers cannot be removed without force: container state improper","response":0}"#
			.to_string(),
	}
}

fn inspect(state: &str) -> Reply {
	Reply {
		status: 200,
		body: format!(r#"{{"State":{{"Status":"{state}"}}}}"#),
	}
}

/// Builds the handler of a daemon that refuses the removal with `$status` and
/// says the container is `$state` when it is asked.
macro_rules! refusing {
	($name:ident, $status:expr, $state:expr) => {
		fn $name(line: &str, _: &str) -> Option<Reply> {
			if line.starts_with("GET") && line.contains("/containers/json") {
				return Some(listing());
			}
			if line.starts_with("GET") && line.contains("proj-web-1/json") {
				return Some(inspect($state));
			}
			if line.starts_with("DELETE") && line.contains("proj-web-1") {
				return Some(refused($status));
			}
			None
		}
	};
}

refusing!(running_500, 500, "running");
refusing!(running_409, 409, "running");
refusing!(paused_500, 500, "paused");
refusing!(exited_500, 500, "exited");

/// The daemon refuses the removal and cannot be asked about the container.
fn refusing_and_silent(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("proj-web-1/json") {
		return Some(Reply {
			status: 500,
			body: r#"{"cause":"boom","message":"cannot inspect","response":500}"#.to_string(),
		});
	}
	if line.starts_with("GET") && line.contains("/containers/json") {
		return Some(listing());
	}
	if line.starts_with("DELETE") && line.contains("proj-web-1") {
		return Some(refused(500));
	}
	None
}

fn rm(handler: fake_engine::Handler, args: &[&str]) -> (Option<i32>, String, FakeEngine) {
	let engine = FakeEngine::start(handler);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	let mut full = vec!["-f", "compose.yaml", "-p", "proj", "rm"];
	full.extend_from_slice(args);
	let (code, output) = engine.run_with_status(None, &[], &full);
	(code, output, engine)
}

fn deletes(engine: &FakeEngine) -> Vec<String> {
	engine
		.sent()
		.lines()
		.filter(|l| l.starts_with("DELETE"))
		.map(str::to_string)
		.collect()
}

#[test]
fn a_running_container_is_skipped_whatever_status_the_refusal_has() {
	for (name, handler) in [
		("500", running_500 as fake_engine::Handler),
		("409", running_409),
	] {
		let (code, output, _) = rm(handler, &["web"]);
		assert_eq!(code, Some(0), "{name}: {output}");
		assert!(
			output.contains("proj-web-1 is running; skipping (pass -f to force removal)"),
			"{name}: {output}"
		);
	}
}

#[test]
fn a_paused_container_is_skipped_and_called_paused() {
	let (code, output, _) = rm(paused_500, &["web"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(
		output.contains("proj-web-1 is paused; skipping (pass -f to force removal)"),
		"{output}"
	);
}

#[test]
fn a_refusal_for_a_container_that_is_not_running_stays_an_error() {
	let (code, output, _) = rm(exited_500, &["web"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(output.contains("container state improper"), "{output}");
}

#[test]
fn a_refusal_that_cannot_be_explained_stays_an_error() {
	let (code, output, _) = rm(refusing_and_silent, &["web"]);
	assert!(matches!(code, Some(c) if c != 0), "{output}");
	assert!(!output.contains("skipping"), "{output}");
}

#[test]
fn with_force_the_removal_is_asked_for_and_the_state_is_never_looked_up() {
	let (_, _, engine) = rm(running_500, &["-f", "web"]);
	assert!(
		deletes(&engine).iter().any(|l| l.contains("force=true")),
		"{:?}",
		deletes(&engine)
	);
	assert!(
		engine
			.sent()
			.lines()
			.all(|l| !l.contains("proj-web-1/json")),
		"{}",
		engine.sent()
	);
}
