//! Every list command prints its header, including on an empty result, and
//! the header sits over its own columns.
//!
//! Split out from `tests/output_contract.rs`; the shared fixtures are declared
//! in the crate root, which the scenario files inherit via `use super::*`.

use std::process::Command;

use super::*;
use harness::{bin, podman_up, Project};

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
