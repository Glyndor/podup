use super::*;
use crate::compose::types::Service;

fn bare_service() -> Service {
	let file = parse_str(
		"services:\n  a:\n    image: x\n    environment: [BARE_FROM_DOTENV, BARE_FROM_FILE, BARE_ABSENT, FIXED=a]\n",
	)
	.expect("compose parses");
	file.services["a"].clone()
}

#[test]
fn a_bare_key_reads_the_project_dotenv() {
	let base = temp_base();
	std::fs::write(base.path().join(".env"), "BARE_FROM_DOTENV=18081\n").unwrap();
	let env = super::super::effective_env(&bare_service(), base.path(), &[]).unwrap();
	assert_eq!(
		env.get("BARE_FROM_DOTENV").map(String::as_str),
		Some("18081")
	);
	assert_eq!(env.get("FIXED").map(String::as_str), Some("a"));
}

#[test]
fn a_bare_key_stays_undefined_when_nothing_defines_it() {
	let base = temp_base();
	let env = super::super::effective_env(&bare_service(), base.path(), &[]).unwrap();
	assert!(!env.contains_key("BARE_ABSENT"));
	assert!(!env.contains_key("BARE_FROM_DOTENV"));
}

#[test]
fn a_bare_key_reads_the_env_files_that_replace_dotenv() {
	let base = temp_base();
	std::fs::write(base.path().join(".env"), "BARE_FROM_DOTENV=from-dotenv\n").unwrap();
	std::fs::write(base.path().join("a.env"), "BARE_FROM_FILE=from-a\n").unwrap();
	let files = [base.path().join("a.env").to_string_lossy().into_owned()];
	let env = super::super::effective_env(&bare_service(), base.path(), &files).unwrap();
	assert_eq!(
		env.get("BARE_FROM_FILE").map(String::as_str),
		Some("from-a")
	);
	assert!(!env.contains_key("BARE_FROM_DOTENV"));
}

#[test]
fn the_process_environment_wins_over_dotenv_for_a_bare_key() {
	let base = temp_base();
	std::fs::write(base.path().join(".env"), "PATH=from-dotenv\n").unwrap();
	let file = parse_str("services:\n  a:\n    image: x\n    environment: [PATH]\n").unwrap();
	let env = super::super::effective_env(&file.services["a"], base.path(), &[]).unwrap();
	assert_eq!(env.get("PATH"), std::env::var("PATH").ok().as_ref());
}

#[test]
fn a_null_map_value_reads_the_project_dotenv_like_a_bare_key() {
	let base = temp_base();
	std::fs::write(base.path().join(".env"), "MAP_FROM_DOTENV=7\n").unwrap();
	let file = parse_str(
		"services:\n  a:\n    image: x\n    environment:\n      MAP_FROM_DOTENV:\n      MAP_ABSENT:\n",
	)
	.unwrap();
	let env = super::super::effective_env(&file.services["a"], base.path(), &[]).unwrap();
	assert_eq!(env.get("MAP_FROM_DOTENV").map(String::as_str), Some("7"));
	assert!(!env.contains_key("MAP_ABSENT"));
}

fn unresolved(yaml: &str, base: &std::path::Path) -> Vec<String> {
	let file = parse_str(yaml).expect("compose parses");
	let service = &file.services["a"];
	let env = super::super::effective_env(service, base, &[]).unwrap();
	let spec_env: std::collections::HashMap<String, String> = env.into_iter().collect();
	super::super::unresolved_env_keys(service, &spec_env)
}

#[test]
fn a_bare_key_nothing_defines_is_sent_for_removal_and_the_defined_ones_are_not() {
	let base = temp_base();
	std::fs::write(base.path().join(".env"), "BARE_FROM_DOTENV=1\n").unwrap();
	let keys = unresolved(
		"services:\n  a:\n    image: x\n    environment: [ZED_ABSENT, BARE_FROM_DOTENV, FIXED=a, EMPTY=, ALPHA_ABSENT]\n",
		base.path(),
	);
	assert_eq!(keys, vec!["ALPHA_ABSENT", "ZED_ABSENT"]);
}

#[test]
fn a_null_in_a_mapping_is_removed_unless_something_resolves_it() {
	let base = temp_base();
	let yaml = "services:\n  a:\n    image: x\n    environment:\n      GONE:\n      FOUND:\n      SET: v\n";
	assert_eq!(unresolved(yaml, base.path()), vec!["FOUND", "GONE"]);
	std::fs::write(base.path().join(".env"), "FOUND=1\n").unwrap();
	assert_eq!(unresolved(yaml, base.path()), vec!["GONE"]);
}

/// A bare key in `environment:` overrides the same key in an `env_file`, as in
/// Docker Compose (`docker compose config` prints it as `null`), so the file's
/// value does not come back and the key is one of the removed. A key only the
/// file defines is not in `environment:` at all and is left alone.
#[test]
fn a_bare_key_overrides_the_env_file_and_one_only_in_the_file_is_kept() {
	let base = temp_base();
	std::fs::write(base.path().join("a.env"), "FROM_FILE=1\nONLY_FILE=2\n").unwrap();
	let keys = unresolved(
		"services:\n  a:\n    image: x\n    env_file: [a.env]\n    environment: [FROM_FILE]\n",
		base.path(),
	);
	assert_eq!(keys, vec!["FROM_FILE"]);
}

#[test]
fn a_service_with_no_environment_removes_nothing() {
	let base = temp_base();
	assert!(unresolved("services:\n  a:\n    image: x\n", base.path()).is_empty());
}
