//! `stats` against samples a daemon should never send: a null network map, byte
//! counters that overflow a sum, an out-of-range float, a string where a number
//! belongs and a cut-off frame. Each must end with a defined exit and no panic;
//! the shapes libpod does send must still print their row.

// The fake engine is a Unix domain socket server.
#![cfg(unix)]

use std::fs;

#[path = "../support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

/// Per-case sample bodies, picked by the container name in the request.
const SAMPLES: &[(&str, &str)] = &[
	("valid", "\"CPU\":1.5,\"MemUsage\":1024,\"MemLimit\":2048,\"MemPerc\":50.0,\"PIDs\":3,\"Network\":{\"eth0\":{\"RxBytes\":10,\"TxBytes\":20}}"),
	("null network", "\"CPU\":1.5,\"MemUsage\":1024,\"MemLimit\":2048,\"MemPerc\":50.0,\"PIDs\":3,\"Network\":null"),
	("rx sum overflows", "\"CPU\":1.5,\"MemUsage\":1024,\"MemLimit\":2048,\"MemPerc\":50.0,\"PIDs\":3,\"Network\":{\"a\":{\"RxBytes\":18446744073709551615,\"TxBytes\":0},\"b\":{\"RxBytes\":1,\"TxBytes\":0}}"),
	("float out of range", "\"CPU\":1e999,\"MemUsage\":1024,\"MemLimit\":2048,\"MemPerc\":50.0,\"PIDs\":3,\"Network\":null"),
	("counter is negative", "\"CPU\":1.5,\"MemUsage\":-1,\"MemLimit\":2048,\"MemPerc\":50.0,\"PIDs\":3,\"Network\":null"),
	("counter is a string", "\"CPU\":1.5,\"MemUsage\":\"lots\",\"MemLimit\":2048,\"MemPerc\":50.0,\"PIDs\":3,\"Network\":null"),
];

/// The case number in a request: the project is `leak<N>`, and the container
/// `leak<N>-web-1` carries it into the stats URL.
fn case_of(line: &str) -> Option<usize> {
	line.split("leak")
		.nth(1)?
		.chars()
		.take_while(char::is_ascii_digit)
		.collect::<String>()
		.parse()
		.ok()
}

fn engine() -> FakeEngine {
	FakeEngine::start(|line, _| {
		if !line.starts_with("GET") {
			return None;
		}
		let n = case_of(line)?;
		if line.contains("/containers/json") {
			return Some(Reply {
				status: 200,
				body: format!(
					"[{{\"Id\":\"c\",\"Names\":[\"leak{n}-web-1\"],\"Image\":\"nginx\",\
					 \"ImageID\":\"i\",\"Status\":\"Up\",\"State\":\"running\",\
					 \"Created\":\"2026-01-01T00:00:00Z\",\"Labels\":{{\"podup.project\":\"leak{n}\",\
					 \"podup.service\":\"web\"}},\"Ports\":[],\"StartedAt\":1}}]"
				),
			});
		}
		if line.contains("/containers/stats") {
			let body =
				match SAMPLES.get(n) {
					Some((_, sample)) => {
						format!("{{\"Error\":null,\"Stats\":[{{\"Name\":\"leak{n}-web-1\",{sample}}}]}}\n")
					}
					// The cut-off frame.
					None => format!("{{\"Stats\":[{{\"Name\":\"leak{n}-web-1\",\"CPU\":1."),
				};
			return Some(Reply { status: 200, body });
		}
		None
	})
}

/// Run `stats --no-stream` for one case, as the exit code and the output.
fn stats(case: usize, format: &[&str]) -> (Option<i32>, String) {
	let fake = engine();
	fs::write(
		fake.dir().join("compose.yaml"),
		"services:\n  web:\n    image: nginx\n",
	)
	.unwrap();
	let project = format!("leak{case}");
	let mut args = vec![
		"-f",
		"compose.yaml",
		"-p",
		&project,
		"stats",
		"--no-stream",
		"--no-trunc",
	];
	args.extend_from_slice(format);
	fake.run_with_status(None, &[], &args)
}

/// Index of the case whose frame is cut off mid-number.
const TRUNCATED: usize = SAMPLES.len();

/// Control: the shapes libpod does send print their row, in the table and as
/// JSON that parses.
#[test]
fn valid_and_null_network_samples_print_their_row() {
	for case in [0, 1] {
		let (code, text) = stats(case, &[]);
		assert_eq!(code, Some(0), "{}: {text}", SAMPLES[case].0);
		assert!(
			text.contains(&format!("leak{case}-web-1")),
			"{}: no row\n{text}",
			SAMPLES[case].0
		);
		let (code, json) = stats(case, &["--format", "json"]);
		assert_eq!(code, Some(0), "{}: {json}", SAMPLES[case].0);
		let line = json
			.lines()
			.find(|l| l.trim_start().starts_with('{') || l.trim_start().starts_with('['))
			.unwrap_or("");
		assert!(
			serde_json::from_str::<serde_json::Value>(line).is_ok()
				|| serde_json::from_str::<serde_json::Value>(json.trim()).is_ok(),
			"{}: output is not JSON\n{json}",
			SAMPLES[case].0
		);
	}
}

/// A byte sum that does not fit a u64 is reported with the container's name,
/// never wrapped into a smaller number and never a panic.
#[test]
fn an_overflowing_byte_sum_is_reported_by_name() {
	let (code, text) = stats(2, &[]);
	assert!(!text.contains("panicked"), "{text}");
	assert!(
		code.is_some_and(|c| c != 0),
		"the sum overflows, so it must fail\n{text}"
	);
	assert!(
		text.contains("leak2-web-1"),
		"the error must name the container\n{text}"
	);
}

/// A float outside the range, a negative counter, a string counter and a
/// cut-off frame are errors with an exit code and a reason, not success and not
/// a panic.
#[test]
fn malformed_samples_fail_without_panicking() {
	for case in [3, 4, 5, TRUNCATED] {
		let (code, text) = stats(case, &[]);
		let name = SAMPLES.get(case).map_or("cut-off frame", |s| s.0);
		assert!(!text.contains("panicked"), "{name}: {text}");
		assert!(
			code.is_some_and(|c| c != 0),
			"{name}: a malformed sample must not succeed\n{text}"
		);
		assert!(text.contains("error"), "{name}: no reason given\n{text}");
	}
}
