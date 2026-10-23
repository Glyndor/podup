//! The same-file cases again, parsed from disk, so the on-disk resolver's
//! in-file recursion and merge path runs rather than the in-memory one.
//!
//! Split out of `tests/parse/extends/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

/// Write `yaml` to a `docker-compose.yml` in a fresh tempdir and parse it from
/// disk, returning `(dir, parse_result)`. The on-disk path exercises
/// `resolve_all_extends`/`resolve_one_extends` rather than the in-memory parser.
fn parse_file_yaml(
	yaml: &str,
) -> (
	tempfile::TempDir,
	podup::Result<podup::compose::types::ComposeFile>,
) {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("docker-compose.yml");
	std::fs::write(&path, yaml).unwrap();
	let res = parse_files_with_env_files(&[path], &[]);
	(dir, res)
}

#[test]
fn extends_same_file_chain_from_disk() {
	// A same-file `extends:` (no `file:`) parsed FROM DISK takes the on-disk
	// resolver's in-file recursion + merge path, not just the in-memory one.
	let (_dir, res) = parse_file_yaml(
		"services:\n  base:\n    image: alpine\n    environment:\n      ROOT: \"1\"\n  \
		 mid:\n    extends: base\n    environment:\n      MID: \"1\"\n  \
		 app:\n    extends: mid\n    environment:\n      APP: \"1\"\n",
	);
	let file = res.unwrap();
	let env = file.services["app"].environment.to_map();
	// The whole chain merged: app sees its own, mid's, and base's variables.
	assert_eq!(
		env.get("ROOT").and_then(|v| v.clone()).as_deref(),
		Some("1")
	);
	assert_eq!(env.get("MID").and_then(|v| v.clone()).as_deref(), Some("1"));
	assert_eq!(env.get("APP").and_then(|v| v.clone()).as_deref(), Some("1"));
}

#[test]
fn extends_same_file_self_reference_from_disk_errors() {
	let (_dir, res) = parse_file_yaml("services:\n  a:\n    image: alpine\n    extends: a\n");
	let err = res.unwrap_err();
	assert!(err.to_string().contains("extends itself"), "got: {err}");
}

#[test]
fn extends_same_file_unknown_target_from_disk_errors() {
	let (_dir, res) = parse_file_yaml("services:\n  a:\n    image: alpine\n    extends: ghost\n");
	let err = res.unwrap_err();
	assert!(
		err.to_string().contains("unknown service 'ghost'"),
		"got: {err}"
	);
}
