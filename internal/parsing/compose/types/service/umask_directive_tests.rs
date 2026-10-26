//! Scalar mask directives across file merges and extends.

use crate::compose::{parse_files_with_env_files_interp, parse_str};

fn merged_mask(directive: &str, extends: bool) -> Option<String> {
	if extends {
		let file = parse_str(&format!("services:\n  base:\n    image: alpine\n    umask: '0077'\n  web:\n    extends: base\n    umask: {directive}\n")).unwrap();
		let same_file = file.services["web"].umask.clone();
		let dir = tempfile::tempdir().unwrap();
		let base = dir.path().join("base.yml");
		let child = dir.path().join("child.yml");
		std::fs::write(
			&base,
			"services:\n  base:\n    image: alpine\n    umask: '0077'\n",
		)
		.unwrap();
		std::fs::write(&child, format!("services:\n  web:\n    extends: {{file: base.yml, service: base}}\n    umask: {directive}\n")).unwrap();
		let external = parse_files_with_env_files_interp(&[child], &[], false).unwrap();
		assert_eq!(external.services["web"].umask, same_file);
		return same_file;
	}
	let dir = tempfile::tempdir().unwrap();
	let base = dir.path().join("base.yml");
	let over = dir.path().join("over.yml");
	std::fs::write(
		&base,
		"services:\n  web:\n    image: alpine\n    umask: '0077'\n",
	)
	.unwrap();
	std::fs::write(
		&over,
		format!("services:\n  web:\n    umask: {directive}\n"),
	)
	.unwrap();
	parse_files_with_env_files_interp(&[base, over], &[], false)
		.unwrap()
		.services["web"]
		.umask
		.clone()
}

#[test]
fn umask_reset_clears_across_files() {
	for value in ["!reset null", "!reset 0"] {
		assert_eq!(merged_mask(value, false), None);
	}
}

#[test]
fn umask_override_replaces_across_files() {
	assert_eq!(merged_mask("!override null", false), None);
	assert_eq!(
		merged_mask("!override '0022'", false).as_deref(),
		Some("0022")
	);
}

#[test]
fn umask_reset_clears_through_extends() {
	for value in ["!reset null", "!reset 0"] {
		assert_eq!(merged_mask(value, true), None);
	}
}

#[test]
fn umask_override_replaces_through_extends() {
	assert_eq!(merged_mask("!override null", true), None);
	assert_eq!(
		merged_mask("!override '0022'", true).as_deref(),
		Some("0022")
	);
}

#[test]
fn umask_reset_clears_without_a_base() {
	for value in ["!reset 0", "!reset '0077'", "!reset '${MASK}'"] {
		let file = parse_str(&format!(
			"services:\n  web:\n    image: alpine\n    umask: {value}\n"
		))
		.unwrap();
		assert_eq!(file.services["web"].umask, None);
	}
}

#[test]
fn umask_reset_clears_new_service_in_second_file() {
	let dir = tempfile::tempdir().unwrap();
	let base = dir.path().join("base.yml");
	let over = dir.path().join("over.yml");
	std::fs::write(&base, "services:\n  base:\n    image: alpine\n").unwrap();
	std::fs::write(
		&over,
		"services:\n  web:\n    image: alpine\n    umask: !reset 0\n",
	)
	.unwrap();
	let file = parse_files_with_env_files_interp(&[base, over], &[], true).unwrap();
	assert_eq!(file.services["web"].umask, None);
}

#[test]
fn umask_reset_base_stays_unset_through_extends() {
	let dir = tempfile::tempdir().unwrap();
	let base = dir.path().join("base.yml");
	let child = dir.path().join("child.yml");
	std::fs::write(
		&base,
		"services:\n  base:\n    image: alpine\n    umask: !reset 0\n",
	)
	.unwrap();
	std::fs::write(
		&child,
		"services:\n  web:\n    extends: {file: base.yml, service: base}\n",
	)
	.unwrap();
	let file = parse_files_with_env_files_interp(&[child], &[], true).unwrap();
	assert_eq!(file.services["web"].umask, None);
	let file = parse_str(
		"services:\n  base:\n    image: alpine\n    umask: !reset 0\n  web:\n    extends: base\n",
	)
	.unwrap();
	assert_eq!(file.services["web"].umask, None);
}

#[test]
fn umask_override_wins_without_a_base() {
	let file =
		parse_str("services:\n  web:\n    image: alpine\n    umask: !override '0022'\n").unwrap();
	assert_eq!(file.services["web"].umask.as_deref(), Some("0022"));
}
