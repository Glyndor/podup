//! The interpolation budget belongs to the project, not to each file.
//!
//! One file may expand to at most 16 MiB of substituted text. When that budget
//! restarted for every file, a project spread over 40 files of 14 MiB each was
//! accepted and held about 4 GB resident, while the same text in one file was
//! refused in under a second. Each case spreads the same 14 MiB-per-file load
//! over many files, through `include:`, several `-f` and `extends: {file:}`,
//! and expects the same refusal the single file gets.

use std::fs;
use std::path::Path;

use super::file_process::{command, run, Outcome};

const MIB: usize = 1024 * 1024;
/// Resident ceiling for a refusal. The unfixed loader passes 4 GB.
const RSS_CEILING_KIB: u64 = 512 * 1024;
/// Entries of 1 MiB per file, under the 16 MiB a single file may expand to.
const ENTRIES_PER_FILE: usize = 14;
const FILES: usize = 40;

/// A project directory whose `.env` holds `BIG`, a variable of 1 MiB.
fn project() -> tempfile::TempDir {
	let dir = tempfile::tempdir().expect("create temp dir");
	fs::write(
		dir.path().join(".env"),
		format!("BIG={}\n", "x".repeat(MIB)),
	)
	.unwrap();
	dir
}

/// A service named `name` whose command repeats `${BIG}` `entries` times.
fn service(name: &str, entries: usize) -> String {
	let items = vec!["\"${BIG}\""; entries].join(", ");
	format!("  {name}:\n    image: alpine\n    command: [{items}]\n")
}

fn write(dir: &Path, name: &str, contents: &str) {
	fs::write(dir.join(name), contents).unwrap();
}

/// `count` files, each with one service of `entries` references. The root
/// compose file is written by `root`.
fn spread(dir: &Path, count: usize, entries: usize) {
	for index in 0..count {
		let body = service(&format!("s{index}"), entries);
		write(
			dir,
			&format!("f{index}.yaml"),
			&format!("services:\n{body}"),
		);
	}
}

fn names(count: usize) -> Vec<String> {
	(0..count).map(|index| format!("f{index}.yaml")).collect()
}

fn by_include(dir: &Path, count: usize, entries: usize) -> Outcome {
	spread(dir, count, entries);
	let list = names(count).join(", ");
	write(dir, "compose.yaml", &format!("include: [{list}]\n"));
	run(command(dir, &["config"]))
}

fn by_several_files(dir: &Path, count: usize, entries: usize) -> Outcome {
	spread(dir, count, entries);
	write(dir, "compose.yaml", "services: {}\n");
	let mut args = Vec::new();
	let files = names(count);
	for file in &files {
		args.extend(["-f", file.as_str()]);
	}
	args.push("config");
	run(command(dir, &args))
}

fn by_extends(dir: &Path, count: usize, entries: usize) -> Outcome {
	spread(dir, count, entries);
	let mut compose = String::from("services:\n");
	for index in 0..count {
		compose.push_str(&format!(
			"  e{index}:\n    extends: {{file: f{index}.yaml, service: s{index}}}\n"
		));
	}
	write(dir, "compose.yaml", &compose);
	run(command(dir, &["config"]))
}

fn assert_refused(what: &str, outcome: &Outcome) {
	assert!(!outcome.timed_out, "{what}: ran past the deadline");
	assert!(
		!outcome.success,
		"{what}: {FILES} files of 14 MiB were accepted"
	);
	assert!(
		outcome.text.contains("BIG") && outcome.text.contains("bytes of substituted output"),
		"{what}: the refusal does not name the variable and the budget:\n{}",
		outcome.text
	);
	assert!(
		outcome.rss_kib < RSS_CEILING_KIB,
		"{what}: {} KiB resident before the refusal",
		outcome.rss_kib
	);
}

fn assert_accepted(what: &str, outcome: &Outcome) {
	assert!(
		outcome.success && !outcome.timed_out,
		"{what}: a project under the budget was refused:\n{}",
		outcome.text
	);
}

#[test]
fn includes_share_one_interpolation_budget() {
	let dir = project();
	assert_refused("include", &by_include(dir.path(), FILES, ENTRIES_PER_FILE));
}

#[test]
fn several_dash_f_files_share_one_interpolation_budget() {
	let dir = project();
	assert_refused("-f", &by_several_files(dir.path(), FILES, ENTRIES_PER_FILE));
}

#[test]
fn extends_files_share_one_interpolation_budget() {
	let dir = project();
	assert_refused("extends", &by_extends(dir.path(), FILES, ENTRIES_PER_FILE));
}

/// The control: the same shapes under the budget still load, so the refusals
/// above come from the cap and not from the layout. 15 MiB over five files.
#[test]
fn a_project_under_the_budget_still_loads_through_each_route() {
	let (count, entries) = (5, 3);
	let dir = project();
	assert_accepted("include", &by_include(dir.path(), count, entries));
	let dir = project();
	assert_accepted("-f", &by_several_files(dir.path(), count, entries));
	let dir = project();
	assert_accepted("extends", &by_extends(dir.path(), count, entries));
}

/// Just past the budget in total, spread so that no file is near it.
#[test]
fn a_project_just_over_the_budget_is_refused() {
	let dir = project();
	let outcome = by_include(dir.path(), 17, 1);
	assert!(
		!outcome.success && !outcome.timed_out,
		"17 MiB was accepted"
	);
	assert!(
		outcome.text.contains("bytes of substituted output"),
		"{}",
		outcome.text
	);
}
