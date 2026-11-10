//! Bidirectional and zero-width characters in names must not reach the terminal raw.
//!
//! A right-to-left override in a container name makes the text after it read
//! backwards, so `web-evil` can be shown as something the operator trusts, and
//! a zero-width joiner hides a difference between two names. Names, images,
//! labels and values come from a compose file, from the command line and from
//! the engine's own answers, and podup prints them in tables, progress lines and
//! errors. Each case puts one payload with every family of these characters
//! into one place, runs commands against a fake engine and requires that none
//! of them reaches stdout or stderr as the character itself.

// The fake engine is a Unix domain socket server.
#![cfg(unix)]

use std::fs;

#[path = "../support/fake_engine.rs"]
mod fake_engine;

#[path = "../support/terminal.rs"]
mod terminal;

use fake_engine::{FakeEngine, Reply};
use terminal::{is_invisible_reorder, quoted, raw_bidi, BIDI_PAYLOAD};

const FIELDS: &[(&str, &str)] = &[
	("container_name", "    container_name: {P}\n"),
	("hostname", "    hostname: {P}\n"),
	("labels", "    labels:\n      k: {P}\n      {P}: v\n"),
	("environment", "    environment:\n      K: {P}\n"),
	("command", "    command: [{P}]\n"),
	("profiles", "    profiles: [{P}]\n"),
	("depends_on", "    depends_on: [{P}]\n"),
	("volumes", "    volumes: [{P}]\n"),
	("restart", "    restart: {P}\n"),
	("ports", "    ports: [{P}]\n"),
	("networks", "    networks: [{P}]\n"),
];

const COMMANDS: &[&[&str]] = &[
	&["config"],
	&["config", "--format", "json"],
	&["up", "-d"],
	&["create"],
	&["down"],
	&["ps", "-a"],
	&["images"],
	&["logs", "web"],
	&["stop"],
	&["rm", "-f"],
];

/// A container as the engine lists it, every text field carrying the payload.
fn hostile_list(line: &str, _: &str) -> Option<Reply> {
	if !line.starts_with("GET") || !line.contains("/containers/json") {
		return None;
	}
	let p = quoted(BIDI_PAYLOAD);
	let body = format!(
		"[{{\"Id\":{p},\"Names\":[{p}],\"Image\":{p},\"ImageID\":{p},\"Status\":{p},\
		 \"State\":\"running\",\"Labels\":{{\"podup.project\":\"leak\",\
		 \"podup.service\":\"web\",\"k\":{p}}},\"Ports\":[],\"StartedAt\":1}}]"
	);
	Some(Reply { status: 200, body })
}

fn run_in(fake: &FakeEngine, compose: &str, args: &[&str]) -> String {
	fs::write(fake.dir().join("compose.yaml"), compose).unwrap();
	let mut full = vec!["-f", "compose.yaml", "-p", "leak"];
	full.extend_from_slice(args);
	fake.run(None, &[], &full)
}

#[test]
fn no_hostile_compose_field_prints_a_bidi_or_zero_width_character() {
	let fake = FakeEngine::start(hostile_list);
	let mut found = Vec::new();
	for (name, field) in FIELDS {
		let compose = format!(
			"services:\n  web:\n    image: nginx\n{}",
			field.replace("{P}", &quoted(BIDI_PAYLOAD))
		);
		for args in COMMANDS {
			let text = run_in(&fake, &compose, args);
			if let Some(shown) = raw_bidi(&text) {
				found.push(format!("{name} / podup {}: {shown}", args.join(" ")));
			}
		}
	}
	assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn engine_answers_with_bidi_text_are_escaped_in_listings() {
	let fake = FakeEngine::start(hostile_list);
	let compose = "services:\n  web:\n    image: nginx\n";
	let mut found = Vec::new();
	for args in COMMANDS {
		let text = run_in(&fake, compose, args);
		if let Some(shown) = raw_bidi(&text) {
			found.push(format!("podup {}: {shown}", args.join(" ")));
		}
	}
	assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn command_line_values_with_bidi_text_are_not_echoed_raw() {
	let fake = FakeEngine::start(hostile_list);
	fs::write(
		fake.dir().join("compose.yaml"),
		"services:\n  web:\n    image: nginx\n",
	)
	.unwrap();
	let p = BIDI_PAYLOAD;
	let mut found = Vec::new();
	for rest in [
		vec!["-p", p, "config"],
		vec!["--profile", p, "config"],
		vec!["-p", "leak", "logs", p],
		vec!["-p", "leak", "up", "-d", p],
		vec!["-p", "leak", "stop", p],
		vec!["-p", "leak", "exec", p, "true"],
		vec!["-p", "leak", "port", p, "80"],
		vec!["-p", "leak", "cp", p, "/tmp/x"],
	] {
		let mut args = vec!["-f", "compose.yaml"];
		args.extend(rest);
		let text = fake.run(None, &[], &args);
		if let Some(shown) = raw_bidi(&text) {
			found.push(format!("podup {args:?}: {shown}"));
		}
	}
	assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_listing_really_prints_the_hostile_names() {
	// The control: if `ps` printed no row, the scans above would pass for the
	// wrong reason. The visible part of the name has to be there.
	let fake = FakeEngine::start(hostile_list);
	let text = run_in(
		&fake,
		"services:\n  web:\n    image: nginx\n",
		&["ps", "-a"],
	);
	assert!(
		text.contains("evil"),
		"ps did not print the hostile container; the control is empty:\n{text}"
	);
}

#[test]
fn the_scan_sees_each_family_and_ignores_plain_text() {
	for c in [
		'\u{202e}', '\u{2066}', '\u{2069}', '\u{200b}', '\u{200d}', '\u{feff}', '\u{61c}',
		'\u{200e}',
	] {
		assert!(is_invisible_reorder(c), "{c:?}");
		assert!(raw_bidi(&format!("a{c}b")).is_some(), "{c:?}");
	}
	assert_eq!(raw_bidi("plain \u{e9}\u{4e2d}\u{5b57} text"), None);
	assert_eq!(raw_bidi("\\u202e escaped"), None);
}
