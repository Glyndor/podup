//! The JSON keys each `--format json` command emits, and their types.
//!
//! Split out from `tests/cli/output/output_contract.rs`; the shared fixtures are declared
//! in the crate root, which the scenario files inherit via `use super::*`.

use super::*;
use harness::{podman_up, Project};

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
