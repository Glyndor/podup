//! `env_file:` entries written with `format: raw`: every value as written.

use super::*;
use crate::compose::types::EnvFileEntry;

fn raw(dir: &std::path::Path, text: &str) -> Result<HashMap<String, String>> {
	std::fs::write(dir.join("r.env"), text).unwrap();
	load_env_file_entries(
		&[EnvFileEntry::Config {
			path: "r.env".into(),
			required: None,
			format: Some("raw".into()),
		}],
		dir,
	)
}

#[test]
fn quotes_dollar_signs_spaces_and_comments_are_kept_as_written() {
	let dir = tempfile::tempdir().unwrap();
	let m = raw(
		dir.path(),
		"A=\"quoted\"\nB='single $HOME'\nC=$HOME ${HOME}\nF=val # comment\nG=  spaced  \nH=a=b\nEMPTY=\n# comment\n\n",
	)
	.unwrap();
	assert_eq!(m["A"], "\"quoted\"");
	assert_eq!(m["B"], "'single $HOME'");
	assert_eq!(m["C"], "$HOME ${HOME}");
	assert_eq!(m["F"], "val # comment");
	assert_eq!(m["G"], "  spaced  ");
	assert_eq!(m["H"], "a=b");
	assert_eq!(m["EMPTY"], "");
	assert_eq!(m.len(), 7);
}

#[test]
fn a_bare_key_passes_through_from_the_host_in_raw_mode_too() {
	std::env::set_var("PODUP_RAW_BARE", "host");
	let dir = tempfile::tempdir().unwrap();
	let m = raw(dir.path(), "PODUP_RAW_BARE\n").unwrap();
	assert_eq!(m["PODUP_RAW_BARE"], "host");
}

#[test]
fn a_quote_does_not_continue_the_value_onto_the_next_line() {
	let dir = tempfile::tempdir().unwrap();
	let m = raw(dir.path(), "ML=\"line1\nOTHER=x\"\n").unwrap();
	assert_eq!(m["ML"], "\"line1");
	assert_eq!(m["OTHER"], "x\"");
}

#[test]
fn a_key_with_whitespace_is_an_error_like_docker_compose() {
	let dir = tempfile::tempdir().unwrap();
	assert!(raw(dir.path(), "export X=1\n").is_err());
}

#[test]
fn a_space_before_the_equals_sign_and_a_missing_name_are_errors_like_docker_compose() {
	let dir = tempfile::tempdir().unwrap();
	assert!(raw(dir.path(), "K = v\n").is_err());
	assert!(raw(dir.path(), "=value\nA=1\n").is_err());
	assert_eq!(raw(dir.path(), " K=v\n").unwrap()["K"], "v");
}

#[test]
fn the_default_format_still_strips_quotes() {
	// The control: the same text, no `format:`, is parsed as dotenv.
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("d.env"), "A=\"quoted\"\n").unwrap();
	let m = load_env_file_entries(&[EnvFileEntry::Path("d.env".into())], dir.path()).unwrap();
	assert_eq!(m["A"], "quoted");
}

#[test]
fn a_raw_entry_is_not_interpolated_and_a_dotenv_entry_beside_it_is() {
	std::env::set_var("PODUP_EFR_HOME", "/home/zz");
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("r.env"), "R=$PODUP_EFR_HOME/raw\n").unwrap();
	std::fs::write(dir.path().join("d.env"), "D=$PODUP_EFR_HOME/dot\nE=${R}\n").unwrap();
	let m = load_env_file_entries(
		&[
			EnvFileEntry::Config {
				path: "r.env".into(),
				required: None,
				format: Some("raw".into()),
			},
			EnvFileEntry::Path("d.env".into()),
		],
		dir.path(),
	)
	.unwrap();
	assert_eq!(m["R"], "$PODUP_EFR_HOME/raw");
	assert_eq!(m["D"], "/home/zz/dot");
	assert_eq!(
		m["E"], "$PODUP_EFR_HOME/raw",
		"a raw value is the text a later value sees"
	);
}

#[test]
fn a_control_character_in_a_raw_line_does_not_reach_the_error() {
	let dir = tempfile::tempdir().unwrap();
	for text in ["=\u{1b}[31mvalue\n", "A\u{1b}[31m B=1\n", "A\tB=1\n"] {
		let text = raw(dir.path(), text).unwrap_err().to_string();
		assert!(!text.contains('\u{1b}') && !text.contains('\t'), "{text:?}");
	}
}
