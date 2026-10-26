//! Every fuzz target is built, and every one that is built is run.
//!
//! `fuzz.yml` hands the weekly job an explicit list of target names. A target
//! that has a source file and a `[[bin]]` but is missing from that list is
//! compiled by nobody and fuzzed by nothing, and it looks exactly like a
//! target that found nothing. The reverse is just as quiet: a name in the list
//! with no `[[bin]]` fails the job, but only on the next Monday.
//!
//! Three sets must be the same set: the files in `fuzz/fuzz_targets/`, the
//! `[[bin]]` names in `fuzz/Cargo.toml`, and the `targets` the workflow passes.
//! The comparison is exercised on planted input first, so a rule that could
//! not fail would show up here instead of passing in silence.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_yaml::Value;

fn root() -> &'static Path {
	Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// File stems of the `.rs` files in `dir`.
fn target_files(dir: &Path) -> BTreeSet<String> {
	fs::read_dir(dir)
		.unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
		.filter_map(|entry| entry.ok().map(|e| e.path()))
		.filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
		.filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
		.collect()
}

/// The `name` of every `[[bin]]` table in a Cargo manifest.
fn bin_names(manifest: &str) -> BTreeSet<String> {
	let mut names = BTreeSet::new();
	let mut in_bin = false;
	for line in manifest.lines().map(str::trim) {
		if line.starts_with('[') {
			in_bin = line == "[[bin]]";
		} else if in_bin {
			if let Some(value) = line.strip_prefix("name") {
				let name = value.trim_start().trim_start_matches('=').trim();
				names.insert(name.trim_matches('"').to_string());
			}
		}
	}
	names
}

/// The target names the workflow passes to the reusable fuzz job.
fn workflow_targets(workflow: &str) -> Result<BTreeSet<String>, String> {
	let doc: Value = serde_yaml::from_str(workflow).map_err(|e| format!("not YAML: {e}"))?;
	let list = doc
		.get("jobs")
		.and_then(Value::as_mapping)
		.into_iter()
		.flat_map(|jobs| jobs.values())
		.filter_map(|job| job.get("with")?.get("targets")?.as_str())
		.next()
		.ok_or("no job passes a `targets` list")?;
	let names: Vec<String> =
		serde_json::from_str(list).map_err(|e| format!("`targets` is not a JSON array: {e}"))?;
	Ok(names.into_iter().collect())
}

/// What is wrong when the three sets differ, one line per difference.
fn differences(
	files: &BTreeSet<String>,
	bins: &BTreeSet<String>,
	listed: &BTreeSet<String>,
) -> Vec<String> {
	let mut out = Vec::new();
	for (what, set, other_what, other) in [
		(
			"a file in fuzz/fuzz_targets",
			files,
			"a [[bin]] in fuzz/Cargo.toml",
			bins,
		),
		(
			"a [[bin]] in fuzz/Cargo.toml",
			bins,
			"a file in fuzz/fuzz_targets",
			files,
		),
		(
			"a file in fuzz/fuzz_targets",
			files,
			"a target in fuzz.yml",
			listed,
		),
		(
			"a target in fuzz.yml",
			listed,
			"a file in fuzz/fuzz_targets",
			files,
		),
	] {
		for name in set.difference(other) {
			out.push(format!("`{name}` is {what} but not {other_what}"));
		}
	}
	out
}

#[test]
fn every_fuzz_target_is_built_and_scheduled() {
	let files = target_files(&root().join("fuzz/fuzz_targets"));
	let bins =
		bin_names(&fs::read_to_string(root().join("fuzz/Cargo.toml")).expect("read manifest"));
	let workflow =
		fs::read_to_string(root().join(".github/workflows/fuzz.yml")).expect("read fuzz.yml");
	let listed = workflow_targets(&workflow).expect("fuzz.yml lists its targets");
	assert!(
		files.len() >= 7,
		"only {} fuzz targets were found; the reader has stopped seeing the directory",
		files.len()
	);
	let problems = differences(&files, &bins, &listed);
	assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}

fn set(names: &[&str]) -> BTreeSet<String> {
	names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn matching_sets_have_no_differences() {
	let all = set(&["a", "b"]);
	assert!(differences(&all, &all, &all).is_empty());
}

#[test]
fn a_target_missing_from_the_workflow_is_reported() {
	let problems = differences(&set(&["a", "b"]), &set(&["a", "b"]), &set(&["a"]));
	assert_eq!(
		problems,
		["`b` is a file in fuzz/fuzz_targets but not a target in fuzz.yml"]
	);
}

#[test]
fn a_target_without_a_bin_is_reported() {
	let problems = differences(&set(&["a", "b"]), &set(&["a"]), &set(&["a", "b"]));
	assert_eq!(
		problems,
		["`b` is a file in fuzz/fuzz_targets but not a [[bin]] in fuzz/Cargo.toml"]
	);
}

#[test]
fn a_workflow_name_with_no_file_is_reported() {
	let problems = differences(&set(&["a"]), &set(&["a"]), &set(&["a", "ghost"]));
	assert_eq!(
		problems,
		["`ghost` is a target in fuzz.yml but not a file in fuzz/fuzz_targets"]
	);
}

#[test]
fn bin_names_reads_only_bin_tables() {
	let manifest = "[package]\nname = \"podup-fuzz\"\n\n[[bin]]\nname = \"one\"\npath = \"x.rs\"\n\n[dependencies.podup]\npath = \"..\"\n\n[[bin]]\nname = \"two\"\n";
	assert_eq!(bin_names(manifest), set(&["one", "two"]));
}

#[test]
fn the_workflow_list_is_read_from_the_job_that_calls_the_reusable() {
	let planted =
		"jobs:\n  fuzz:\n    uses: ./x.yml\n    with:\n      targets: '[\"one\", \"two\"]'\n";
	assert_eq!(workflow_targets(planted), Ok(set(&["one", "two"])));
	assert!(workflow_targets("jobs:\n  fuzz:\n    uses: ./x.yml\n").is_err());
}
