//! The identifier the table prints is the one the other commands accept: the
//! `logs` prefix names the container the same way `ps` does, and the SERVICE
//! column round-trips through both.
//!
//! Split out from `tests/output_contract.rs`; the shared fixtures are declared
//! in the crate root, which the scenario files inherit via `use super::*`.

use std::process::Command;

use super::*;
use harness::{bin, podman_up, Project};

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

/// Property over the surface: every row of `ps` carries at least one
/// identifier `logs` accepts as a target. Not "the SERVICE column round-trips"
/// (that is the example) but "no row leaves the reader with no usable
/// identifier", which holds however the table is laid out.
///
/// The example-based test reads only the SERVICE column, so a future layout
/// shift that drops it would still leave the column-positioned assertion
/// pointing at the same index, while every row would be unreadable. This test
/// walks the cells instead, so a regression that loses the only accepted
/// identifier is caught no matter which column carries it.
///
/// Stated over the cells the command actually printed rather than a hardcoded
/// name: the fixture can rename the service and the contract stays the same.
#[tokio::test]
async fn every_ps_row_has_an_identifier_logs_accepts() {
	if !podman_up().await {
		return;
	}
	let p = Project::start("psprop");
	let ps_out = p.run(&["ps"]);
	let data: Vec<&str> = ps_out
		.lines()
		.filter(|l| !l.trim().is_empty())
		.skip(1) // drop the header
		.collect();
	assert!(!data.is_empty(), "ps printed no data row: {ps_out:?}");

	for row in &data {
		let cells: Vec<&str> = row.split_whitespace().collect();
		// The cells a reader can pick from: every non-empty value, deduped
		// so two cells carrying the same string only probe it once. STATUS
		// ('Up') and CREATED ('5 minutes ago') are common false positives
		// that the reader would never paste back into `logs`.
		let mut tried: Vec<&str> = Vec::new();
		for c in &cells {
			if !c.is_empty() && !tried.contains(c) {
				tried.push(c);
			}
		}
		let mut accepted: Vec<&str> = Vec::new();
		for value in &tried {
			let logs = Command::new(bin())
				.args([
					"-f", &p.compose, "-p", &p.name, "logs", "--tail", "0", value,
				])
				.output()
				.expect("run podup logs <cell>");
			if logs.status.success() {
				accepted.push(value);
			}
		}
		assert!(
			!accepted.is_empty(),
			"ps row carried no identifier accepted by `logs`: {row:?}\nall cells tried: {tried:?}\nfull output:\n{ps_out}"
		);
	}
}
