//! The committed fuzz seeds are used, and every seed belongs to a real target.
//!
//! `fuzz/seeds/<target>/` holds hostile inputs the smoke run starts from. A
//! directory named after a target that no longer exists is read by nothing, and
//! a reusable workflow that stops passing the directory leaves every seed idle.
//! Both look exactly like a fuzz run that found nothing.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn root() -> &'static Path {
	Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Names of the entries in `dir`, files or directories.
fn entry_names(dir: &Path) -> BTreeSet<String> {
	fs::read_dir(dir)
		.unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
		.filter_map(|entry| entry.ok())
		.map(|entry| {
			let name = entry.file_name().to_string_lossy().into_owned();
			name.strip_suffix(".rs").map(str::to_string).unwrap_or(name)
		})
		.collect()
}

/// One line per seed directory that names no target.
fn orphan_seed_dirs(seeds: &BTreeSet<String>, targets: &BTreeSet<String>) -> Vec<String> {
	seeds
		.difference(targets)
		.map(|name| format!("fuzz/seeds/{name} has no fuzz target of that name"))
		.collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
	names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn every_seed_directory_names_a_fuzz_target() {
	let seeds = entry_names(&root().join("fuzz/seeds"));
	let targets = entry_names(&root().join("fuzz/fuzz_targets"));
	assert!(!seeds.is_empty(), "fuzz/seeds holds no directory");
	let problems = orphan_seed_dirs(&seeds, &targets);
	assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}

#[test]
fn every_seed_is_a_nonempty_file() {
	let mut bad = Vec::new();
	for target in entry_names(&root().join("fuzz/seeds")) {
		let dir = root().join("fuzz/seeds").join(&target);
		let files = fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
		let mut count = 0;
		for entry in files.filter_map(|e| e.ok()) {
			count += 1;
			let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
			if !entry.path().is_file() || len == 0 {
				bad.push(format!("{} is not a nonempty file", entry.path().display()));
			}
		}
		if count == 0 {
			bad.push(format!("{} holds no seed", dir.display()));
		}
	}
	assert!(bad.is_empty(), "\n{}\n", bad.join("\n"));
}

#[test]
fn the_fuzz_workflow_hands_the_seeds_to_libfuzzer() {
	let reusable = fs::read_to_string(root().join(".github/workflows/reusable-rust-fuzz.yml"))
		.expect("read reusable-rust-fuzz.yml");
	assert!(
		reusable.contains("fuzz/seeds/$TARGET")
			&& reusable.contains("cargo fuzz run \"$TARGET\" \"${corpus[@]}\""),
		"reusable-rust-fuzz.yml no longer passes fuzz/seeds/<target> to cargo fuzz run"
	);
}

#[test]
fn a_seed_directory_without_a_target_is_reported() {
	let problems = orphan_seed_dirs(&set(&["gone", "dotenv"]), &set(&["dotenv"]));
	assert_eq!(
		problems,
		["fuzz/seeds/gone has no fuzz target of that name"]
	);
}
