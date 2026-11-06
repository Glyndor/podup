//! The project budget holds when the library is entered by a single file.
//!
//! The command line always loads through the list of files, whose own scope
//! would hide a missing one in the single-file entry points. These call
//! `parse_file_with_env_files` and `parse_file_with_env_files_interp`
//! directly, with `include:` files of 1 MiB each: 15 MiB loads, 17 MiB does not.

use std::fs;
use std::path::PathBuf;

use podup::compose::{parse_file_with_env_files, parse_file_with_env_files_interp};

const MIB: usize = 1024 * 1024;

/// A project whose root includes `count` files, each repeating `${BIG}` once.
fn project(count: usize) -> (tempfile::TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("create temp dir");
	fs::write(
		dir.path().join(".env"),
		format!("BIG={}\n", "x".repeat(MIB)),
	)
	.unwrap();
	let mut names = Vec::new();
	for index in 0..count {
		let name = format!("f{index}.yaml");
		let body =
			format!("services:\n  s{index}:\n    image: alpine\n    command: [\"${{BIG}}\"]\n");
		fs::write(dir.path().join(&name), body).unwrap();
		names.push(name);
	}
	let root = dir.path().join("compose.yaml");
	fs::write(&root, format!("include: [{}]\n", names.join(", "))).unwrap();
	(dir, root)
}

fn assert_budget_refusal(error: impl std::fmt::Display) {
	let text = error.to_string();
	assert!(text.contains("bytes of substituted output"), "{text}");
}

#[test]
fn parse_file_with_env_files_shares_the_budget_across_includes() {
	let (_dir, root) = project(15);
	parse_file_with_env_files(&root, &[]).expect("15 MiB is under the budget");
	let (_dir, root) = project(17);
	assert_budget_refusal(parse_file_with_env_files(&root, &[]).expect_err("17 MiB is over"));
}

#[test]
fn parse_file_with_env_files_interp_shares_the_budget_across_includes() {
	let (_dir, root) = project(15);
	parse_file_with_env_files_interp(&root, &[], true).expect("15 MiB is under the budget");
	let (_dir, root) = project(17);
	assert_budget_refusal(
		parse_file_with_env_files_interp(&root, &[], true).expect_err("17 MiB is over"),
	);
}
