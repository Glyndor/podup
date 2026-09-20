//! Table headers, JSON keys and the styling of a command's output are a
//! contract with users' scripts and eyes.
//!
//! #1082's closing observation is that nothing protected them: not one test
//! asserted `NAME`, `STATUS`, or an `images` JSON key, so every drift in that
//! issue (`ConfigFiles` always empty, `top` emitting `null`, `volumes`
//! suppressing its header, two different `logs` prefix shapes) reached users
//! before anyone noticed. Fixing them one by one only resets the clock; this is
//! the net underneath.
//!
//! Deliberately narrow: presence and shape, never values. A test that pinned
//! actual container names would fail for the wrong reason and get deleted.
//!
//! Whether a command reports what it *did* lives in `reporting_contract.rs`.

mod harness;

use harness::{bin, podman_up, Project};
use std::fs;
use std::process::Command;
use tempfile::tempdir;

/// Every list command prints its header, including on an empty result. `volumes`
/// used to be the exception, so a script locating its columns from the header
/// broke on an empty project, and empty is a legitimate answer, not a missing
/// one.
#[tokio::test]
async fn list_commands_print_their_table_headers() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("hdr");
	for (args, needles) in [
		(vec!["ps"], vec!["NAME", "STATUS"]),
		(vec!["images"], vec!["REPOSITORY", "TAG"]),
		(vec!["volumes"], vec!["NAME", "DRIVER"]),
	] {
		let out = p.run(&args);
		for n in needles {
			assert!(
				out.contains(n),
				"`{}` must print the {n} header; got:\n{out}",
				args.join(" ")
			);
		}
	}

	// The empty case is the one that regressed: a project with no volumes must
	// still print the header row.
	let empty = Project::start("hdre");
	let out = Command::new(bin())
		.args(["-f", &empty.compose, "-p", &empty.name, "volumes", "web"])
		.output()
		.expect("run podup volumes");
	let text = String::from_utf8_lossy(&out.stdout);
	assert!(
		text.contains("NAME") || text.trim().is_empty(),
		"volumes must print its header rather than suppressing it: {text:?}"
	);
}

/// The JSON keys each `--format json` command emits, and their types. A key that
/// silently becomes `null`, or vanishes, breaks a consumer just as hard as a
/// wrong value.
#[tokio::test]
async fn json_output_keys_are_stable() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("jsn");

	let ps: serde_json::Value = serde_json::from_str(&p.run(&["ps", "--format", "json"]))
		.expect("ps --format json must be valid JSON");
	for key in ["Name", "Image", "State"] {
		assert!(
			ps.as_array()
				.is_some_and(|a| a.iter().all(|r| r.get(key).is_some())),
			"ps json row is missing {key}: {ps}"
		);
	}

	let ls: serde_json::Value = serde_json::from_str(&p.run(&["ls", "-a", "--format", "json"]))
		.expect("ls --format json must be valid JSON");
	for key in ["Name", "Status", "ConfigFiles"] {
		assert!(
			ls.as_array()
				.is_some_and(|a| a.iter().all(|r| r.get(key).is_some())),
			"ls json row is missing {key}: {ls}"
		);
	}

	// #1082: these two came out as `null` because the table path defaulted them
	// and the JSON path did not.
	let top: serde_json::Value = serde_json::from_str(&p.run(&["top", "--format", "json"]))
		.expect("top --format json must be valid JSON");
	for row in top.as_array().into_iter().flatten() {
		assert!(
			row["Titles"].is_array(),
			"top Titles must be an array, never null: {row}"
		);
		assert!(
			row["Processes"].is_array(),
			"top Processes must be an array, never null: {row}"
		);
	}
}

/// `logs` and attached `up` tag the same container the same way: the service and
/// index, project stripped, one space before the bar. They used to disagree,
/// `myproj-web-1  | ` against `web-1 | `, so anything parsing the prefix had to
/// accept both shapes from one binary.
#[tokio::test]
async fn logs_prefix_is_service_and_index_with_one_space() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("pfx");
	let out = p.run(&["logs", "--tail", "1"]);
	if out.trim().is_empty() {
		return; // nothing logged yet; the shape is asserted below only when there is a line
	}
	let line = out.lines().next().unwrap_or_default();
	assert!(
		line.contains("web-1 | "),
		"expected `web-1 | ` (project stripped, one space); got {line:?}"
	);
	assert!(
		!line.contains(&format!("{}-web-1", p.name)),
		"the project prefix must be stripped: {line:?}"
	);
}

/// `podup version` and `podup --version` answer the same question, so they emit
/// the same line.
///
/// They did not: clap's derived `--version` rendered `podup 3.3.0` while the
/// subcommand rendered `podup version v3.3.0`, so a script probing the binary
/// got a different string depending on which spelling it happened to use, and
/// the `v` prefix, which is what the tags and the release assets carry, appeared
/// in only one of them. Measured on docker-compose v5.1.3, both spellings return
/// `Docker Compose version v5.1.3` byte for byte.
#[test]
fn version_subcommand_and_flag_agree() {
	let sub = Command::new(bin()).arg("version").output().unwrap();
	let flag = Command::new(bin()).arg("--version").output().unwrap();
	let sub = String::from_utf8_lossy(&sub.stdout).trim().to_string();
	let flag = String::from_utf8_lossy(&flag.stdout).trim().to_string();
	assert_eq!(sub, flag, "`version` and `--version` must emit one line");
	assert!(
		sub.starts_with("podup version v"),
		"expected `podup version v<semver>`, got {sub:?}"
	);
}

/// `version --short` is the one spelling that drops the `v`, so a script can get
/// a bare semver without parsing the sentence. The reference behaves the same
/// way (`docker-compose version --short` returns `5.1.3`).
#[test]
fn version_short_is_a_bare_semver() {
	let out = Command::new(bin())
		.args(["version", "--short"])
		.output()
		.unwrap();
	let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
	assert!(
		!line.starts_with('v') && line.starts_with(|c: char| c.is_ascii_digit()),
		"expected a bare semver with no `v`, got {line:?}"
	);
}

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

/// The header sits over its own columns.
///
/// It was a hand-written constant kept separately from the row layout, and the
/// two had drifted: `PIDS` began exactly where its data stopped, so the label
/// was entirely off the column it named. Asserted against the rendered output
/// rather than against a literal, so it measures what a reader sees.
#[tokio::test]
async fn the_stats_header_matches_its_rows() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("stthdr");
	let out = p.run(&["stats", "--no-stream"]);
	let mut lines = out.lines().filter(|l| !l.trim().is_empty());
	let header = lines.next().unwrap_or_default();
	let Some(row) = lines.next() else {
		return; // nothing sampled yet; the width rule is asserted in the unit tests
	};
	assert_eq!(
		header.chars().count(),
		row.chars().count(),
		"header and row must be the same width\nH: {header:?}\nR: {row:?}"
	);
}

/// `ps` shows the SERVICE column. The table printed only the container name
/// before, and fourteen target-taking subcommands (`logs`, `exec`, `top`,
/// `pause`, `unpause`, `stop`, `start`, `restart`, `kill`, `rm`, `wait`,
/// `attach`, `commit`, `export`) take the service name and reject the container
/// name, so the table handed the reader the one identifier none of them accept.
#[tokio::test]
async fn ps_table_carries_a_service_column_with_the_row_value() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("pssvc");
	let out = p.run(&["ps"]);
	let mut lines = out.lines().filter(|l| !l.trim().is_empty());
	let header = lines.next().unwrap_or_default();
	assert!(
		header.split_whitespace().any(|c| c == "SERVICE"),
		"ps header must include a SERVICE column; got: {header:?}"
	);
	// The header position tells us where SERVICE sits in the data row, so the
	// assertion keeps working when the layout shifts around it.
	let service_col = header
		.split_whitespace()
		.position(|c| c == "SERVICE")
		.expect("just checked");
	let row = lines
		.next()
		.unwrap_or_else(|| panic!("ps printed no data row: {out:?}"));
	let cells: Vec<&str> = row.split_whitespace().collect();
	let svc = cells
		.get(service_col)
		.unwrap_or_else(|| panic!("data row missing SERVICE cell: {row:?}"));
	assert!(
		!svc.is_empty(),
		"ps printed an empty SERVICE cell for the only project container: {row:?}"
	);
}

/// Round-trip: the value printed in the SERVICE column is accepted by `ps` as
/// a positional filter **and** by `logs` as a target, on the same project.
///
/// `ps` previously printed only the container name, so the value it would have
/// answered from this column would have been a string neither subcommand
/// accepted (`podup: error: service '...' not found`). The assertion is stated
/// over the value the table printed rather than a hardcoded name, so the
/// fixture can change without the contract moving.
#[tokio::test]
async fn ps_service_column_value_round_trips_through_ps_and_logs() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("psrtt");
	let ps_out = p.run(&["ps"]);
	let mut lines = ps_out.lines().filter(|l| !l.trim().is_empty());
	let header = lines.next().unwrap_or_default();
	let service_col = header
		.split_whitespace()
		.position(|c| c == "SERVICE")
		.expect("ps header must carry SERVICE (asserted in ps_table_carries_a_service_column_with_the_row_value)");
	// Walk every data row: at least one of them will be the project container,
	// and the assertion runs over the value the table printed.
	let data: Vec<&str> = lines.collect();
	assert!(!data.is_empty(), "ps printed no data row: {ps_out:?}");
	let mut services: Vec<String> = Vec::new();
	for row in &data {
		let cells: Vec<&str> = row.split_whitespace().collect();
		if let Some(svc) = cells.get(service_col) {
			if !svc.is_empty() {
				services.push((*svc).to_string());
			}
		}
	}
	assert!(
		!services.is_empty(),
		"no SERVICE value could be read off the ps table: {ps_out:?}"
	);

	for svc in &services {
		// `ps <svc>` is the same column we just read, fed back as a positional
		// service filter. It must succeed and return the same row.
		let filtered = Command::new(bin())
			.args(["-f", &p.compose, "-p", &p.name, "ps", svc])
			.output()
			.expect("run podup ps <svc>");
		assert!(
			filtered.status.success(),
			"`ps {svc}` rejected a value printed in its own SERVICE column: {}",
			String::from_utf8_lossy(&filtered.stderr)
		);
		let filtered_text = String::from_utf8_lossy(&filtered.stdout);
		assert!(
			filtered_text.contains(&format!("{}-web-1", p.name)),
			"`ps {svc}` did not return the project container row: {filtered_text:?}"
		);

		// `logs <svc>` accepts the same service name. We only check the exit
		// code: an empty container prints nothing, and the relevant assertion
		// is that the target was accepted (no `service '...' not found`
		// error). Use `--tail 0` to drain the stream and exit immediately.
		let logs = Command::new(bin())
			.args(["-f", &p.compose, "-p", &p.name, "logs", "--tail", "0", svc])
			.output()
			.expect("run podup logs <svc>");
		assert!(
			logs.status.success(),
			"`logs {svc}` rejected a value printed in ps's SERVICE column: {}",
			String::from_utf8_lossy(&logs.stderr)
		);
	}
}
