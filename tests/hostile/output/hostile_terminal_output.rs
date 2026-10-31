//! Terminal control sequences in names must not reach the terminal raw.
//!
//! Names, images, labels and paths come from a compose file, from the command
//! line and from the engine's own answers, and podup prints them in tables,
//! progress lines and errors. A raw ESC in one of them can retitle the window,
//! repaint earlier output or hide a line. Each case puts a payload made of an
//! OSC title, a CSI clear-screen, a BEL and a backspace into one place, runs
//! commands against a fake engine and requires that none of those bytes reaches
//! stdout or stderr. Colour is off (`NO_COLOR`) and podup's own `ESC [ ... m`
//! styling is removed before the scan, so it cannot be mistaken for the payload.

// The fake engine is a Unix domain socket server.
#![cfg(unix)]

use std::fs;

#[path = "../support/fake_engine.rs"]
mod fake_engine;

#[path = "../support/terminal.rs"]
mod terminal;

use fake_engine::{FakeEngine, Reply};
use terminal::{quoted, raw_control, PAYLOAD};

/// One compose file per hostile field, so a refusal of one does not hide the
/// rest. `{P}` is the quoted payload.
const FIELDS: &[(&str, &str)] = &[
	("container_name", "    container_name: {P}\n"),
	("hostname", "    hostname: {P}\n"),
	("labels", "    labels:\n      k: {P}\n      {P}: v\n"),
	("environment", "    environment:\n      K: {P}\n"),
	("command", "    command: [{P}]\n"),
	("working_dir", "    working_dir: {P}\n"),
	("user", "    user: {P}\n"),
	("profiles", "    profiles: [{P}]\n"),
	("network_mode", "    network_mode: {P}\n"),
	("depends_on", "    depends_on: [{P}]\n"),
	("extends", "    extends:\n      service: {P}\n"),
	("volumes", "    volumes: [{P}]\n"),
	("healthcheck", "    healthcheck:\n      test: [CMD, {P}]\n"),
	("restart", "    restart: {P}\n"),
	("pull_policy", "    pull_policy: {P}\n"),
	("stop_signal", "    stop_signal: {P}\n"),
	("platform", "    platform: {P}\n"),
	("ports", "    ports: [{P}]\n"),
	("cap_add", "    cap_add: [{P}]\n"),
	("secrets", "    secrets: [{P}]\n"),
	("build", "    build:\n      context: {P}\n"),
	("ulimits", "    ulimits:\n      {P}: 1\n"),
	("deploy", "    deploy:\n      replicas: {P}\n"),
];

fn compose_for(field: &str) -> String {
	format!(
		"services:\n  web:\n    image: nginx\n{}",
		field.replace("{P}", &quoted(PAYLOAD))
	)
}

/// A container as the engine lists it, every text field carrying the payload.
fn hostile_list(line: &str, _: &str) -> Option<Reply> {
	if !line.starts_with("GET") || !line.contains("/containers/json") {
		return None;
	}
	let p = quoted(PAYLOAD);
	let body = format!(
		"[{{\"Id\":{p},\"Names\":[{p}],\"Image\":{p},\"ImageID\":{p},\"Status\":{p},\
		 \"State\":\"running\",\"Created\":{p},\"Labels\":{{\"podup.project\":\"leak\",\
		 \"podup.service\":\"web\",\"k\":{p}}},\"Ports\":[],\"StartedAt\":1}}]"
	);
	Some(Reply { status: 200, body })
}

const COMMANDS: &[&[&str]] = &[
	&["config"],
	&["config", "--no-interpolate"],
	&["config", "--format", "json"],
	&["generate", "quadlet"],
	&["audit"],
	&["up", "-d"],
	&["create"],
	&["pull"],
	&["down"],
	&["ps", "-a"],
	&["images"],
	&["top", "web"],
	&["port", "web", "80"],
	&["logs", "web"],
	&["stop"],
	&["restart"],
	&["kill"],
	&["rm", "-f"],
	&["events", "--json"],
];

fn run_in(fake: &FakeEngine, compose: &str, args: &[&str]) -> String {
	fs::write(fake.dir().join("compose.yaml"), compose).unwrap();
	let mut full = vec!["-f", "compose.yaml", "-p", "leak"];
	full.extend_from_slice(args);
	fake.run(None, &[], &full)
}

#[test]
fn no_hostile_compose_field_reaches_the_terminal_raw() {
	let fake = FakeEngine::start(hostile_list);
	let mut found = Vec::new();
	for (name, field) in FIELDS {
		let compose = compose_for(field);
		for args in COMMANDS {
			let text = run_in(&fake, &compose, args);
			if let Some(shown) = raw_control(&text) {
				found.push(format!(
					"{name} / podup {}: raw control in {shown}",
					args.join(" ")
				));
			}
		}
	}
	assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn hostile_engine_answers_are_escaped_in_every_listing() {
	let fake = FakeEngine::start(hostile_list);
	let compose = "services:\n  web:\n    image: nginx\n";
	let mut found = Vec::new();
	for args in COMMANDS {
		let text = run_in(&fake, compose, args);
		if let Some(shown) = raw_control(&text) {
			found.push(format!("podup {}: raw control in {shown}", args.join(" ")));
		}
	}
	assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn hostile_command_line_values_do_not_reach_the_terminal_raw() {
	let fake = FakeEngine::start(hostile_list);
	fs::write(
		fake.dir().join("compose.yaml"),
		"services:\n  web:\n    image: nginx\n",
	)
	.unwrap();
	let p = PAYLOAD;
	let mut cases: Vec<Vec<String>> = vec![
		vec!["-f".into(), p.into(), "config".into()],
		vec![
			"-f".into(),
			"compose.yaml".into(),
			"--env-file".into(),
			p.into(),
			"config".into(),
		],
		vec![
			"-f".into(),
			"compose.yaml".into(),
			"-p".into(),
			p.into(),
			"config".into(),
		],
		vec![
			"-f".into(),
			"compose.yaml".into(),
			"--profile".into(),
			p.into(),
			"config".into(),
		],
		vec![
			"-f".into(),
			"compose.yaml".into(),
			"--project-directory".into(),
			p.into(),
			"config".into(),
		],
		vec![p.into()],
		vec!["--socket".into(), p.into(), "ps".into()],
	];
	for rest in [
		vec!["logs", p],
		vec!["up", "-d", p],
		vec!["stop", p],
		vec!["restart", p],
		vec!["exec", p, "true"],
		vec!["exec", "web", p],
		vec!["run", p],
		vec!["scale", p],
		vec!["port", p, "80"],
		vec!["top", p],
		vec!["cp", p, "/tmp/x"],
		vec!["pull", p],
		vec!["build", p],
		vec!["wait", p],
		vec!["kill", "-s", p],
		vec!["audit", "--format", p],
	] {
		// The values are payloads built at run time, so own them.
		let owned: Vec<String> = ["-f", "compose.yaml", "-p", "leak"]
			.iter()
			.map(|s| s.to_string())
			.chain(rest.iter().map(|s| s.to_string()))
			.collect();
		cases.push(owned);
	}
	let mut found = Vec::new();
	for args in cases {
		let refs: Vec<&str> = args.iter().map(String::as_str).collect();
		let text = fake.run(None, &[], &refs);
		if let Some(shown) = raw_control(&text) {
			found.push(format!("podup {args:?}: raw control in {shown}"));
		}
	}
	assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_listing_really_prints_the_hostile_names() {
	// The control: if `ps` printed no row, the scans above would pass for the
	// wrong reason. The row has to be there with the payload visible as text.
	let fake = FakeEngine::start(hostile_list);
	let text = run_in(
		&fake,
		"services:\n  web:\n    image: nginx\n",
		&["ps", "-a"],
	);
	assert!(
		text.contains("PWNED"),
		"ps did not print the hostile container; the control is empty:\n{text}"
	);
	assert_eq!(raw_control(&text), None);
}

#[test]
fn the_scan_sees_a_raw_escape_and_ignores_podups_own_styling() {
	assert!(raw_control("ok\u{1b}[2J").is_some());
	assert!(raw_control("a\u{1b}]0;t\u{7}").is_some());
	assert!(raw_control("a\u{8}").is_some());
	assert_eq!(raw_control("a\tb\nc"), None);
	assert_eq!(
		raw_control("\u{1b}[1mNAME\u{1b}[0m \u{1b}[38;5;3mx\u{1b}[0m"),
		None
	);
}
