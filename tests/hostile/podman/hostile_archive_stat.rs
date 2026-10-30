//! Malformed archive metadata must stop cp before its archive PUT.
#![cfg(unix)]

use std::fs;

use base64::Engine as _;

#[path = "../support/fake_engine.rs"]
mod fake_engine;
#[path = "../support/host_files.rs"]
mod host_files;

use fake_engine::{FakeEngine, Reply};
use host_files::Running;

fn reply(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("HEAD ") {
		return Some(Reply {
			status: 200,
			body: String::new(),
		});
	}
	None
}

fn missing(line: &str, request: &str) -> Option<Reply> {
	if line.starts_with("HEAD ") {
		return Some(Reply {
			status: 404,
			body: String::new(),
		});
	}
	reply(line, request)
}

fn encoded(json: &str) -> String {
	base64::engine::general_purpose::STANDARD.encode(json)
}

fn check(header: Option<String>, status: u16, diagnostic: &str, accepted: bool) {
	let headers = header
		.map(|h| format!("X-Docker-Container-Path-Stat: {h}\r\n"))
		.unwrap_or_default();
	let fake = FakeEngine::start_with_headers(
		if status == 404 { missing } else { reply },
		// One leaked string per case: the handler API wants a `'static` header block.
		Box::leak(headers.into_boxed_str()),
	);
	fs::write(
		fake.dir().join("compose.yaml"),
		"services:\n  web:\n    image: alpine\n",
	)
	.unwrap();
	fs::write(fake.dir().join("src"), "safe source").unwrap();
	fs::create_dir(fake.dir().join("outside")).unwrap();
	fs::write(fake.dir().join("outside/keep"), host_files::MARKER).unwrap();
	let mut child = Running::start(&fake, &["cp", "src", "web:/dst/"]);
	let result = child.wait();
	let text = child.text();
	assert!(
		result.code().is_some(),
		"terminated by signal: {result}: {text}"
	);
	assert_ne!(result.code(), Some(101), "panic: {text}");
	assert!(!text.contains("panicked at"), "{text}");
	let sent = fake.sent();
	assert!(
		sent.lines()
			.any(|l| l.starts_with("HEAD ") && l.contains("/archive?path=")),
		"HEAD not reached: {sent}\n{text}"
	);
	assert_eq!(fs::read(fake.dir().join("src")).unwrap(), b"safe source");
	assert_eq!(
		fs::read(fake.dir().join("outside/keep")).unwrap(),
		host_files::MARKER.as_bytes()
	);
	assert_eq!(fs::read_dir(fake.dir().join("outside")).unwrap().count(), 1);
	if accepted {
		assert!(result.success(), "valid control failed: {text}");
		assert!(
			sent.lines().any(|l| l.starts_with("PUT ")),
			"no upload: {sent}"
		);
	} else {
		assert!(
			!result.success(),
			"hostile stat was accepted: {sent}\n{text}"
		);
		assert!(text.contains(diagnostic), "missing {diagnostic:?}: {text}");
		assert!(
			!sent.lines().any(|l| l.starts_with("PUT ")),
			"upload after hostile stat: {sent}"
		);
	}
}

#[test]
fn valid_directory_control_reaches_put() {
	check(
		Some(encoded(r#"{"name":"dst","mode":2147484141,"size":0}"#)),
		200,
		"",
		true,
	);
}

#[test]
fn corrupt_base64_is_a_clean_error() {
	check(
		Some("%%%not-base64%%%".into()),
		200,
		"malformed container path stat",
		false,
	);
}

#[test]
fn truncated_json_is_a_clean_error() {
	check(Some(encoded(r#"{"mode":"#)), 200, "EOF", false);
}

#[test]
fn negative_size_is_a_clean_error() {
	check(
		Some(encoded(r#"{"mode":2147484141,"size":-1}"#)),
		200,
		"invalid",
		false,
	);
}

#[test]
fn string_mode_is_a_clean_error() {
	check(
		Some(encoded(r#"{"mode":"directory"}"#)),
		200,
		"invalid type",
		false,
	);
}

#[test]
fn a_numeric_name_is_ignored_since_only_the_mode_decides() {
	check(
		Some(encoded(r#"{"name":42,"mode":2147484141}"#)),
		200,
		"stat",
		true,
	);
}

#[test]
fn an_enormous_size_is_ignored_since_only_the_mode_decides() {
	check(
		Some(encoded(
			r#"{"size":18446744073709551615,"mode":2147484141}"#,
		)),
		200,
		"stat",
		true,
	);
}

#[test]
fn an_absurd_mtime_is_ignored_since_only_the_mode_decides() {
	check(
		Some(encoded(
			r#"{"mtime":"999999999999999999-99-99T99:99:99Z","mode":2147484141}"#,
		)),
		200,
		"stat",
		true,
	);
}

#[test]
fn megabyte_header_is_a_bounded_error() {
	check(
		Some(encoded(&format!(
			r#"{{"mtime":"{}","mode":2147484141}}"#,
			"x".repeat(2 * 1024 * 1024)
		))),
		200,
		"error",
		false,
	);
}

#[test]
fn absent_header_on_200_cannot_claim_a_directory() {
	check(None, 200, "not a directory", false);
}

#[test]
fn missing_directory_on_404_is_a_clean_error() {
	check(None, 404, "no such directory", false);
}

#[test]
fn a_404_ignores_even_a_corrupt_stat_header() {
	check(Some("%%%".into()), 404, "no such directory", false);
}
