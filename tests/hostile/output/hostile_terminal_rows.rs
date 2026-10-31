//! `images`, `top` and `stats` must print a row built from a hostile answer,
//! escaped. The listing sweep in `hostile_terminal_output` answers only
//! `/containers/json`, so those commands got an empty body there and passed
//! without ever reaching the code that renders the engine's text.

// The fake engine is a Unix domain socket server.
#![cfg(unix)]

use std::fs;

#[path = "../support/fake_engine.rs"]
mod fake_engine;
#[path = "../support/terminal.rs"]
mod terminal;

use fake_engine::{FakeEngine, Reply};
use terminal::{quoted, raw_control, PAYLOAD};

/// Every endpoint these commands read, with the payload in the text fields
/// that reach the output.
fn hostile_rows(line: &str, _: &str) -> Option<Reply> {
	if !line.starts_with("GET") {
		return None;
	}
	let p = quoted(PAYLOAD);
	let body = if line.contains("/containers/json") {
		// A running container `web`, named with the payload: `stats` keeps a
		// sample only when its name matches a listed running container.
		format!(
			"[{{\"Id\":\"cid\",\"Names\":[{p}],\"Image\":\"nginx\",\"ImageID\":\"iid\",\
			 \"Status\":\"Up\",\"State\":\"running\",\"Created\":\"2026-01-01T00:00:00Z\",\
			 \"Labels\":{{\"podup.project\":\"leak\",\"podup.service\":\"web\"}},\"Ports\":[],\
			 \"StartedAt\":1}}]"
		)
	} else if line.contains("/images/nginx/json") {
		format!(
			"{{\"Id\":{p},\"RepoTags\":[{p}],\"RepoDigests\":[{p}],\"Size\":10,\
			 \"Created\":{p}}}"
		)
	} else if line.contains("/top") {
		format!("{{\"Titles\":[{p}],\"Processes\":[[{p}]]}}")
	} else if line.contains("/containers/stats") {
		format!(
			"{{\"Error\":null,\"Stats\":[{{\"Name\":{p},\"CPU\":1.5,\"MemUsage\":1024,\
			 \"MemLimit\":2048,\"MemPerc\":50.0,\"BlockInput\":1,\"BlockOutput\":2,\
			 \"PIDs\":3,\"Network\":null}}]}}\n"
		)
	} else {
		return None;
	};
	Some(Reply { status: 200, body })
}

fn run(args: &[&str]) -> String {
	let fake = FakeEngine::start(hostile_rows);
	fs::write(
		fake.dir().join("compose.yaml"),
		"services:\n  web:\n    image: nginx\n",
	)
	.unwrap();
	let mut full = vec!["-f", "compose.yaml", "-p", "leak"];
	full.extend_from_slice(args);
	fake.run(None, &[], &full)
}

/// The control and the scan in one: the row is there with the payload shown as
/// escaped text (`\u{1b}`; a narrow column may cut it before `PWNED`), and no
/// raw control byte came with it.
fn assert_row_is_escaped(args: &[&str]) {
	let text = run(args);
	assert!(
		text.contains("\\u{1b}"),
		"podup {args:?} printed no row built from the hostile answer:\n{text}"
	);
	assert_eq!(raw_control(&text), None, "podup {args:?}");
}

#[test]
fn images_prints_a_hostile_image_escaped() {
	assert_row_is_escaped(&["images"]);
}

#[test]
fn top_prints_a_hostile_process_table_escaped() {
	assert_row_is_escaped(&["top", "web"]);
}

#[test]
fn stats_prints_a_hostile_sample_escaped() {
	assert_row_is_escaped(&["stats", "--no-stream"]);
}
