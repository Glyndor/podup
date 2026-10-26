//! Colour carries meaning: the same answer gets the same colour, the columns a
//! reader scans carry the dimming, and a piped stream stays plain.
//!
//! Split out from `tests/cli/output/output_contract.rs`; the shared fixtures are declared
//! in the crate root, which the scenario files inherit via `use super::*`.

use std::fs;
use std::process::Command;
use tempfile::tempdir;

use super::*;
use harness::{bin, podman_up, Project};

/// `ls` tints the project name, and two different projects get two different
/// colours. It was the only list command whose first column was bare.
///
/// The assertion is on *distinctness*, not on a particular escape sequence: a
/// test that re-derived the expected code from the same palette constant the
/// renderer reads would pass whether or not the renderer consulted it.
#[tokio::test]
async fn ls_tints_each_project_name() {
	if !podman_up().await {
		return;
	}
	let a = Project::start("lsa");
	let b = Project::start("lsb");
	let out = Command::new(bin())
		.args(["-f", &a.compose, "--ansi", "always", "ls"])
		.output()
		.expect("run podup ls");
	let text = String::from_utf8_lossy(&out.stdout);
	let colour_of = |name: &str| -> Option<String> {
		text.lines()
			.find(|l| l.contains(name))
			.and_then(|l| l.split_once(&format!("m{name}")))
			.map(|(head, _)| head.rsplit('\u{1b}').next().unwrap_or("").to_string())
	};
	let (ca, cb) = (colour_of(&a.name), colour_of(&b.name));
	assert!(
		ca.is_some() && cb.is_some(),
		"both project names must carry a colour; got:\n{text}"
	);
	assert_ne!(
		ca, cb,
		"two projects must not share one colour, or the column says nothing:\n{text}"
	);
}

/// `autostart status` gives one answer one colour.
///
/// With no unit file on disk it gave two: `installed: no` was dim while
/// `enabled: not-found` was red, though both report the same fact about the same
/// uninstalled unit and neither is a failure. The red one belonged to the case
/// where nothing is wrong, which is the reading an operator scanning six
/// consecutive yes/no lines will act on first.
///
/// Compares the two lines' value styles against each other rather than against a
/// literal escape sequence, so the test cannot pass by re-deriving what the
/// renderer was going to emit anyway.
#[test]
fn autostart_status_renders_one_negative_one_way() {
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(&compose, "services:\n  web:\n    image: alpine:latest\n").unwrap();
	let out = Command::new(bin())
		.args([
			"-f",
			&compose.to_string_lossy(),
			"-p",
			&format!("t{}-nounit", std::process::id()),
			"--ansi",
			"always",
			"autostart",
			"status",
		])
		.output()
		.expect("run podup autostart status");
	let text = String::from_utf8_lossy(&out.stdout);

	// The style applied to a line's value: everything after the label's own reset.
	let value_style = |label: &str| -> Option<String> {
		text.lines()
			.find(|l| l.contains(&format!("{label}:")))
			.and_then(|l| l.split_once("\u{1b}[0m "))
			.map(|(_, rest)| {
				// The whole leading SGR sequence, terminator included. Stopping at
				// the first non-alphanumeric char instead truncates `\x1b[2m` and
				// `\x1b[31m` to the same `\x1b[`, which made an earlier version of
				// this test pass with the control deleted.
				let mut style = String::new();
				for c in rest.chars() {
					style.push(c);
					if c == 'm' {
						break;
					}
				}
				style
			})
	};

	let installed = value_style("installed");
	let enabled = value_style("enabled");
	assert!(
		installed.is_some() && enabled.is_some(),
		"both lines must be present; got:\n{text}"
	);
	assert_eq!(
		installed, enabled,
		"`installed: no` and `enabled: not-found` report the same uninstalled \
		 unit, so they must not be styled differently; got:\n{text}"
	);
}

/// `top`'s process rows carry styling, and the dimming really reaches them.
///
/// Two header lines were styled and every process row was flat, which on a busy
/// container is the whole output. The bookkeeping columns are now dimmed so the
/// command line is what the eye lands on.
///
/// This is deliberately an end-to-end assertion rather than a unit test of the
/// column choice. Both exist, because they fail for different reasons: the unit
/// test catches a wrong choice, and only this one catches the choice being
/// computed correctly and then not passed to the table, which is exactly the
/// shape of the `logs` defect that survived every per-task review in #1247.
#[tokio::test]
async fn top_styles_its_process_rows() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("topsty");
	let out = Command::new(bin())
		.args([
			"-f", &p.compose, "-p", &p.name, "--ansi", "always", "top", "web",
		])
		.output()
		.expect("run podup top");
	let text = String::from_utf8_lossy(&out.stdout);
	let process_rows: Vec<&str> = text
		.lines()
		.filter(|l| l.contains("sleep") || l.contains("root"))
		.collect();
	assert!(
		!process_rows.is_empty(),
		"expected at least one process row; got:\n{text}"
	);
	assert!(
		process_rows.iter().any(|l| l.contains("\u{1b}[2m")),
		"process rows must carry the dim styling, not just the two headers; got:\n{text}"
	);
}

/// A piped `stats` stays a readable transcript, and its machine paths are never
/// repainted.
///
/// The live view is gated on **stdout** rather than stderr, because `stats` is
/// its own output: `stats > file` on a terminal must still produce a file of
/// frames rather than a file of cursor moves. `--format json` never repaints at
/// all, whatever the terminal says, and keeps the split it already had: NDJSON
/// while streaming, one pretty array for `--no-stream`.
#[tokio::test]
async fn piped_stats_is_frames_and_json_never_repaints() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("stt");

	let table = p.run(&["stats", "--no-stream"]);
	assert!(
		!table.contains('\u{1b}'),
		"a piped stats table must carry no escapes; got:\n{table:?}"
	);
	assert!(
		table.contains("NAME") && table.contains("PIDS"),
		"and it must still be the table; got:\n{table}"
	);

	let json = p.run(&["stats", "--no-stream", "--format", "json"]);
	assert!(
		!json.contains('\u{1b}'),
		"the machine path must carry no escapes; got:\n{json:?}"
	);
	let parsed: serde_json::Value =
		serde_json::from_str(&json).expect("--no-stream --format json stays one pretty document");
	assert!(parsed.is_array(), "and it stays an array: {parsed}");
}
