//! `!reset null` on a top-level entry removes it from the earlier files.

use std::path::PathBuf;

const BASE: &str = "networks: {n1: {}, n2: {}}\n\
volumes: {v1: {}, v2: {}}\n\
secrets: {s1: {file: ./s1}, s2: {file: ./s2}}\n\
services:\n  s: {image: busybox, volumes: [\"v2:/x\"], networks: [n2]}\n  t: {image: busybox}\n";

fn merged(over: &str) -> crate::compose::types::ComposeFile {
	let dir = tempfile::tempdir().unwrap();
	let base: PathBuf = dir.path().join("base.yaml");
	let over_path: PathBuf = dir.path().join("over.yaml");
	std::fs::write(&base, BASE).unwrap();
	std::fs::write(&over_path, over).unwrap();
	super::parse_files_with_env_files(&[base, over_path], &[]).unwrap()
}

#[test]
fn reset_removes_a_service_from_the_base() {
	let file = merged("services:\n  t: !reset null\n");
	assert!(file.services.contains_key("s"));
	assert!(!file.services.contains_key("t"));
}

#[test]
fn reset_removes_a_network_and_a_volume_from_the_base() {
	let file = merged("networks: {n1: !reset null}\nvolumes: {v1: !reset null}\n");
	assert!(!file.networks.contains_key("n1"));
	assert!(file.networks.contains_key("n2"));
	assert!(!file.volumes.contains_key("v1"));
	assert!(file.volumes.contains_key("v2"));
}

#[test]
fn reset_removes_a_secret_from_the_base() {
	let file = merged("secrets: {s1: !reset null}\n");
	assert!(!file.secrets.contains_key("s1"));
	assert!(file.secrets.contains_key("s2"));
}

#[test]
fn an_entry_that_is_not_reset_is_kept() {
	let file = merged("networks: {n1: {}}\nservices:\n  t: {image: other}\n");
	assert!(file.networks.contains_key("n1"));
	assert_eq!(file.services["t"].image.as_deref(), Some("other"));
}

#[test]
fn reset_in_a_single_file_leaves_no_entry() {
	let file = super::parse_str(
		"networks: {n1: !reset null}\nservices:\n  t: !reset null\n  u: {image: x}\n",
	)
	.unwrap();
	assert!(!file.networks.contains_key("n1"));
	assert!(!file.services.contains_key("t"));
	assert!(file.services.contains_key("u"));
}

/// A reset in the middle of three files is undone by a later file that
/// declares the entry again.
#[test]
fn a_later_file_can_declare_a_reset_entry_again() {
	let dir = tempfile::tempdir().unwrap();
	let files: Vec<PathBuf> = ["a", "b", "c"]
		.iter()
		.zip([
			"services:\n  t: {image: one}\n",
			"services:\n  t: !reset null\n",
			"services:\n  t: {image: three}\n",
		])
		.map(|(name, text)| {
			let path = dir.path().join(format!("{name}.yaml"));
			std::fs::write(&path, text).unwrap();
			path
		})
		.collect();
	let file = super::parse_files_with_env_files(&files, &[]).unwrap();
	assert_eq!(file.services["t"].image.as_deref(), Some("three"));
}
