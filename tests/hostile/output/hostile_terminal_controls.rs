//! Carriage return, C1 controls and NUL in names must not reach the terminal raw.

// The fake engine is a Unix domain socket server.
#![cfg(unix)]

#[path = "../support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

const PAYLOAD: &str = "A\r\u{9b}2J\u{0}B";

fn quoted(s: &str) -> String {
	let mut out = String::from("\"");
	for c in s.chars() {
		if c.is_control() {
			out.push_str(&format!("\\u{:04x}", c as u32));
		} else {
			out.push(c);
		}
	}
	out.push('"');
	out
}

fn list(line: &str, _: &str) -> Option<Reply> {
	if !line.starts_with("GET") || !line.contains("/containers/json") {
		return None;
	}
	let p = quoted(PAYLOAD);
	let body = format!(
		"[{{\"Id\":{p},\"Names\":[{p}],\"Image\":{p},\"Status\":{p},\"State\":\"running\",\
		 \"Labels\":{{\"podup.project\":\"leak\",\"podup.service\":\"web\"}},\"Ports\":[],\
		 \"StartedAt\":1}}]"
	);
	Some(Reply { status: 200, body })
}

fn run(fake: &FakeEngine, args: &[&str]) -> String {
	std::fs::write(
		fake.dir().join("compose.yaml"),
		format!(
			"services:\n  web:\n    image: nginx\n    container_name: {}\n",
			quoted(PAYLOAD)
		),
	)
	.unwrap();
	let mut full = vec!["-f", "compose.yaml", "-p", "leak"];
	full.extend_from_slice(args);
	fake.run(None, &[], &full)
}

#[test]
fn progress_rows_and_warnings_do_not_print_a_carriage_return_c1_or_nul() {
	let fake = FakeEngine::start(list);
	let mut found = Vec::new();
	for args in [
		&["up", "-d"][..],
		&["down"],
		&["stop"],
		&["restart"],
		&["kill"],
		&["rm", "-f"],
	] {
		let text = run(&fake, args);
		if text
			.chars()
			.any(|c| c.is_control() && c != '\n' && c != '\t' && c != '\u{1b}')
		{
			found.push(format!("podup {}: {text:?}", args.join(" ")));
		}
	}
	assert!(found.is_empty(), "{}", found.join("\n"));
}
