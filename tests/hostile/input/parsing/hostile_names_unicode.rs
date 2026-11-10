//! Names that look like an ordinary one but are not must be refused.
//!
//! A right-to-left override, an isolate, a zero-width character or a Cyrillic
//! letter that looks like a Latin one makes two names read the same, or makes
//! one read as another. A project name and a service name are limited to ASCII
//! letters, digits, `.`, `_` and `-`, and the keys of the top-level `networks`,
//! `volumes`, `secrets` and `configs` have to be ASCII. Each case puts one such
//! character into one name, runs `config` and `up` against a fake engine and requires a
//! refusal that names the problem, no request that creates anything, and no
//! invisible or reordering character in the message.

// The fake engine is a Unix domain socket server.
#![cfg(unix)]

use std::fs;

#[path = "../../support/fake_engine.rs"]
mod fake_engine;

#[path = "../../support/terminal.rs"]
mod terminal;

use fake_engine::{FakeEngine, Reply};
use terminal::raw_bidi;

/// One character of each family: override, isolate, zero-width space and
/// joiner, byte order mark, and a Cyrillic `a` (U+0430).
const CHARS: &[&str] = &[
	"\\u202e", "\\u2066", "\\u200b", "\\u200d", "\\ufeff", "\\u0430",
];

/// A file with the hostile character `{C}` in the name of one resource.
const SHAPES: &[(&str, &str)] = &[
	("service", "services:\n  \"w{C}\":\n    image: nginx\n"),
	(
		"network",
		"services:\n  web:\n    image: nginx\nnetworks:\n  \"n{C}\": {}\n",
	),
	(
		"volume",
		"services:\n  web:\n    image: nginx\nvolumes:\n  \"v{C}\": {}\n",
	),
	(
		"secret",
		"services:\n  web:\n    image: nginx\nsecrets:\n  \"s{C}\":\n    environment: X\n",
	),
	(
		"config",
		"services:\n  web:\n    image: nginx\nconfigs:\n  \"c{C}\":\n    environment: X\n",
	),
];

fn quiet(_: &str, _: &str) -> Option<Reply> {
	None
}

/// Whether the engine was asked to create, change or remove anything.
fn changed_something(sent: &str) -> bool {
	sent.lines().any(|l| {
		["POST ", "PUT ", "DELETE "]
			.iter()
			.any(|m| l.starts_with(m))
	})
}

fn run(fake: &FakeEngine, compose: &str, args: &[&str]) -> (Option<i32>, String) {
	fs::write(fake.dir().join("compose.yaml"), compose).unwrap();
	let mut full = vec!["-f", "compose.yaml", "-p", "leak"];
	full.extend_from_slice(args);
	fake.run_with_status(None, &[], &full)
}

#[test]
fn a_look_alike_resource_name_is_refused_by_config_and_up() {
	let mut bad = Vec::new();
	for (kind, shape) in SHAPES {
		for c in CHARS {
			let compose = shape.replace("{C}", c);
			for args in [&["config", "-q"][..], &["up", "-d"]] {
				let fake = FakeEngine::start(quiet);
				let (code, text) = run(&fake, &compose, args);
				let label = format!("{kind} {c} / podup {}", args.join(" "));
				if code == Some(0) {
					bad.push(format!("{label}: accepted"));
				}
				if let Some(shown) = raw_bidi(&text) {
					bad.push(format!("{label}: raw character in {shown}"));
				}
				if changed_something(&fake.sent()) {
					bad.push(format!("{label}: the engine was asked to change something"));
				}
			}
		}
	}
	assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_look_alike_project_name_is_refused_from_every_source() {
	let fake = FakeEngine::start(quiet);
	let compose = "services:\n  web:\n    image: nginx\n";
	let named = "name: \"x\\u202ey\"\nservices:\n  web:\n    image: nginx\n";
	fs::write(fake.dir().join("compose.yaml"), compose).unwrap();
	fs::write(fake.dir().join("named.yaml"), named).unwrap();
	let mut bad = Vec::new();
	for name in ["x\u{202e}y", "x\u{200b}y", "x\u{2066}y", "p\u{430}y"] {
		let by_flag = fake.run_with_status(
			None,
			&[],
			&["-f", "compose.yaml", "-p", name, "config", "-q"],
		);
		let by_variable = fake.run_with_status(
			None,
			&[("COMPOSE_PROJECT_NAME", name)],
			&["-f", "compose.yaml", "config", "-q"],
		);
		for (source, (code, text)) in [("flag", by_flag), ("variable", by_variable)] {
			if code == Some(0) {
				bad.push(format!("{source} {name:?}: accepted"));
			}
			if let Some(shown) = raw_bidi(&text) {
				bad.push(format!("{source} {name:?}: raw character in {shown}"));
			}
		}
	}
	let (code, text) = fake.run_with_status(None, &[], &["-f", "named.yaml", "config", "-q"]);
	assert_ne!(
		code,
		Some(0),
		"a `name:` with an override was accepted:\n{text}"
	);
	assert_eq!(raw_bidi(&text), None);
	assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn the_control_accepts_a_plain_ascii_name() {
	// If every file were refused, the cases above would pass for the wrong
	// reason.
	let fake = FakeEngine::start(quiet);
	let ok = "services:\n  web:\n    image: nginx\nnetworks:\n  n.1_a-b: {}\nvolumes:\n  v: {}\n";
	let (code, text) = run(&fake, ok, &["config", "-q"]);
	assert_eq!(code, Some(0), "{text}");
}
