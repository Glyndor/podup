//! The `up` flags that change what is shown and asked of the log stream:
//! `--timestamps` and `--quiet-pull`. The engine is a fake Unix socket, so
//! nothing here touches Podman.

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

/// No image is in local storage, so it is pulled; a log request is answered
/// with one framed line.
fn no_images(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("/images/") && line.contains("/json") {
		return Some(Reply {
			status: 404,
			body: r#"{"message":"no such image"}"#.to_string(),
		});
	}
	if line.contains("/logs") {
		return Some(Reply {
			status: 200,
			body: "\u{1}\0\0\0\0\0\0\u{6}hello\n".into(),
		});
	}
	None
}

fn engine() -> FakeEngine {
	let engine = FakeEngine::start(no_images);
	fs::write(engine.dir().join("compose.yaml"), COMPOSE).unwrap();
	engine
}

fn up(engine: &FakeEngine, args: &[&str]) -> (Option<i32>, String) {
	let mut full = vec!["-f", "compose.yaml", "-p", "proj", "up"];
	full.extend_from_slice(args);
	engine.run_with_status(None, &[], &full)
}

fn log_request(engine: &FakeEngine) -> String {
	engine
		.sent()
		.lines()
		.find(|l| l.starts_with("GET") && l.contains("/logs?"))
		.unwrap_or_else(|| panic!("no log stream was opened:\n{}", engine.sent()))
		.to_string()
}

fn pulls(engine: &FakeEngine) -> usize {
	engine
		.sent()
		.lines()
		.filter(|l| l.contains("/images/pull"))
		.count()
}

#[test]
fn timestamps_are_asked_of_the_log_stream_only_with_the_flag() {
	let engine = engine();
	let (_, output) = up(&engine, &[]);
	assert!(
		log_request(&engine).contains("timestamps=false"),
		"{output}"
	);

	let engine = self::engine();
	let (_, output) = up(&engine, &["--timestamps"]);
	assert!(log_request(&engine).contains("timestamps=true"), "{output}");
}

#[test]
fn quiet_pull_hides_the_pull_progress_but_still_pulls() {
	let engine = engine();
	let (code, output) = up(&engine, &["-d"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(output.contains("Image web-image"), "control: {output}");
	let pulled = pulls(&engine);
	assert!(pulled >= 1, "the image is pulled: {}", engine.sent());

	let engine = self::engine();
	let (code, output) = up(&engine, &["-d", "--quiet-pull"]);
	assert_eq!(code, Some(0), "{output}");
	assert!(!output.contains("Image web-image"), "{output}");
	assert_eq!(pulls(&engine), pulled, "the image is still pulled");
}
