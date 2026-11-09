//! The Rust CI jobs build with settings that keep `target/` small.
//!
//! `reusable-rust-ci.yml` sets `CARGO_INCREMENTAL=0` and reduces the debug
//! info of the dev and test profiles to line tables at workflow level, so every
//! job inherits them. Dropping either line brings back caches several times
//! larger with no failing test to say so.

use std::fs;
use std::path::Path;

use serde_yaml::Value;

fn workflow_env() -> Value {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/reusable-rust-ci.yml");
	let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
	let doc: Value = serde_yaml::from_str(&text).expect("reusable-rust-ci.yml is YAML");
	doc.get("env")
		.cloned()
		.expect("the workflow has a top-level env")
}

#[test]
fn incremental_compilation_is_off_in_ci() {
	assert_eq!(
		workflow_env()
			.get("CARGO_INCREMENTAL")
			.and_then(Value::as_str),
		Some("0")
	);
}

#[test]
fn dev_and_test_builds_keep_line_tables_only() {
	assert_eq!(
		workflow_env()
			.get("CARGO_PROFILE_DEV_DEBUG")
			.and_then(Value::as_str),
		Some("line-tables-only")
	);
}

#[test]
fn no_job_overrides_the_build_settings_back() {
	let text = fs::read_to_string(
		Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/reusable-rust-ci.yml"),
	)
	.expect("read workflow");
	for name in [
		"CARGO_INCREMENTAL",
		"CARGO_PROFILE_DEV_DEBUG",
		"CARGO_PROFILE_TEST_DEBUG",
	] {
		let sets = text
			.lines()
			.filter(|l| l.trim_start().starts_with(&format!("{name}:")))
			.count();
		assert!(
			sets <= 1,
			"{name} is set {sets} times; a job-level value would undo the workflow-level one"
		);
	}
}
