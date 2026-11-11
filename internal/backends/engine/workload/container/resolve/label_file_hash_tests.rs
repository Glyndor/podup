//! `config_hash` and `label_file:`: the labels a container carries come from
//! the file's contents, so editing the file has to change the hash and make
//! `up` recreate the container, the way an `env_file:` edit does.

use super::config_hash;
use crate::parse_str;
use std::collections::{BTreeMap, HashMap};

const PROJECT: &str = "proj";

fn hash_of(base: &std::path::Path) -> String {
	let file = parse_str("services:\n  web:\n    image: x\n    label_file: labels.env\n").unwrap();
	let service = file.services.get("web").unwrap();
	config_hash(
		service,
		&file,
		PROJECT,
		base,
		&HashMap::new(),
		&BTreeMap::new(),
	)
	.unwrap()
}

#[test]
fn editing_a_label_file_changes_the_hash() {
	let base = tempfile::tempdir().unwrap();
	std::fs::write(base.path().join("labels.env"), "tier=one\n").unwrap();
	let before = hash_of(base.path());
	std::fs::write(base.path().join("labels.env"), "tier=one\nnew=label\n").unwrap();
	assert_ne!(before, hash_of(base.path()));
}

#[test]
fn comments_and_blank_lines_in_a_label_file_do_not_change_the_hash() {
	let base = tempfile::tempdir().unwrap();
	std::fs::write(base.path().join("labels.env"), "tier=one\n").unwrap();
	let before = hash_of(base.path());
	std::fs::write(base.path().join("labels.env"), "# note\n\ntier=one\n").unwrap();
	assert_eq!(before, hash_of(base.path()));
}

#[test]
fn a_missing_label_file_does_not_make_the_hash_fail() {
	// The create path skips an unreadable `label_file:` with a warning, so
	// hashing must not turn that into an error.
	let base = tempfile::tempdir().unwrap();
	assert!(!hash_of(base.path()).is_empty());
}
