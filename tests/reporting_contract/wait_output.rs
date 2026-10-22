//! `wait` names the container each exit code belongs to, one line per container,
//! and offers a machine path.
//!
//! Split out from `tests/reporting_contract.rs`; the shared fixtures are
//! declared in the crate root, which the scenario files inherit via
//! `use super::*`.

use std::fs;
use tempfile::tempdir;

use super::*;
use harness::{podman_up, Project};

/// `wait` names the container each exit code belongs to, one line per container,
/// and offers a machine path.
///
/// It printed a bare `0` per service. With more than one container nothing said
/// which code was whose, and a service scaled to three collapsed to one line.
/// Measured on docker compose v5.1.3 with three replicas: it reports per
/// container, so the granularity here follows the reference; the rendering
/// deliberately does not, since the reference prints a 64-character hex id, the
/// same sentence on every line, and nothing a parser can read.
#[tokio::test]
async fn wait_names_each_container_and_its_code() {
	if !podman_up().await {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		"services:\n  ok:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"exit 0\"]\n  \
		 bad:\n    image: alpine:latest\n    deploy:\n      replicas: 2\n    command: [\"sh\", \
		 \"-c\", \"exit 3\"]\n",
	)
	.unwrap();
	let p = Project {
		compose: compose.to_string_lossy().into_owned(),
		name: format!("t{}-wait", std::process::id()),
		_dir: dir,
	};
	p.run(&["up", "-d"]);

	let table = p.run(&["wait"]);
	assert!(
		table.contains("NAME") && table.contains("EXIT"),
		"wait must print its header; got:\n{table}"
	);
	for needle in ["-ok-1", "-bad-1", "-bad-2"] {
		assert!(
			table.contains(needle),
			"every container gets its own line, including each replica; \
			 missing {needle} in:\n{table}"
		);
	}

	let ndjson = p.run(&["wait", "--format", "json"]);
	let rows: Vec<serde_json::Value> = ndjson
		.lines()
		.filter(|l| !l.trim().is_empty())
		.map(|l| serde_json::from_str(l).expect("each wait json line must parse on its own"))
		.collect();
	assert_eq!(rows.len(), 3, "one NDJSON object per container: {ndjson}");
	for row in &rows {
		assert!(row.get("Container").is_some(), "missing Container: {row}");
		assert!(row.get("ExitCode").is_some(), "missing ExitCode: {row}");
	}
	assert!(
		rows.iter().any(|r| r["ExitCode"] == 3),
		"the failing containers' code must survive: {ndjson}"
	);
}
