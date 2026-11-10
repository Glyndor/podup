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
