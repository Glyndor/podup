//! `env_file:` on an `include:` entry is a path of the file that includes.
//!
//! Compose Specification, include: `env_file` gives the default values for
//! interpolating the included file, and `project_directory` is the base of the
//! paths set *inside* the included file. Docker Compose reads a relative
//! `env_file` from the directory of the file that has the `include:`, whatever
//! `project_directory` says, and that is measured here against its output.

use podup::parse_files_with_env_files;
use std::fs;

fn write(dir: &std::path::Path, name: &str, text: &str) {
	let path = dir.join(name);
	fs::create_dir_all(path.parent().unwrap()).unwrap();
	fs::write(path, text).unwrap();
}

fn project(include_extra: &str) -> tempfile::TempDir {
	let dir = tempfile::tempdir().unwrap();
	write(
		dir.path(),
		"compose.yaml",
		&format!("include:\n  - path: other.yaml\n    env_file: .env.inc\n{include_extra}services:\n  app:\n    image: x\n"),
	);
	write(
		dir.path(),
		"other.yaml",
		"services:\n  inc:\n    image: x\n    environment:\n      V: \"${PODUP_INC_VAR_5:-unset}\"\n",
	);
	dir
}

fn value(dir: &std::path::Path) -> podup::Result<String> {
	let file = parse_files_with_env_files(&[dir.join("compose.yaml")], &[])?;
	Ok(file.services["inc"].environment.to_map()["V"]
		.clone()
		.unwrap())
}

#[test]
fn the_env_file_is_read_from_the_including_directory_with_a_project_directory() {
	let dir = project("    project_directory: ./sub\n");
	write(dir.path(), ".env.inc", "PODUP_INC_VAR_5=root\n");
	fs::create_dir_all(dir.path().join("sub")).unwrap();
	assert_eq!(value(dir.path()).unwrap(), "root");
}

#[test]
fn and_not_from_the_project_directory() {
	let dir = project("    project_directory: ./sub\n");
	write(dir.path(), "sub/.env.inc", "PODUP_INC_VAR_5=sub\n");
	let error = value(dir.path()).expect_err("the file is not where the including file is");
	assert!(error.to_string().contains(".env.inc"), "{error}");
}

#[test]
fn with_both_files_the_including_directory_wins() {
	let dir = project("    project_directory: ./sub\n");
	write(dir.path(), ".env.inc", "PODUP_INC_VAR_5=root\n");
	write(dir.path(), "sub/.env.inc", "PODUP_INC_VAR_5=sub\n");
	assert_eq!(value(dir.path()).unwrap(), "root");
}

#[test]
fn without_a_project_directory_nothing_changes() {
	let dir = project("");
	write(dir.path(), ".env.inc", "PODUP_INC_VAR_5=root\n");
	assert_eq!(value(dir.path()).unwrap(), "root");
}

#[test]
fn dev_null_as_the_env_file_of_an_include_means_no_env_file() {
	// Docker Compose skips it; it is how an include keeps the including
	// project's `.env` out of the included file.
	let dir = tempfile::tempdir().unwrap();
	write(
		dir.path(),
		"compose.yaml",
		"include:\n  - path: other.yaml\n    env_file: /dev/null\nservices:\n  app:\n    image: x\n",
	);
	write(
		dir.path(),
		"other.yaml",
		"services:\n  inc:\n    image: x\n    environment:\n      V: \"${PODUP_INC_VAR_6:-unset}\"\n",
	);
	assert_eq!(value(dir.path()).unwrap(), "unset");
}

#[test]
fn a_control_character_in_the_env_file_of_an_include_does_not_reach_the_message() {
	let dir = tempfile::tempdir().unwrap();
	write(
		dir.path(),
		"compose.yaml",
		"include:\n  - path: other.yaml\n    env_file: \"no\\e[31mfile.env\"\nservices:\n  app:\n    image: x\n",
	);
	write(
		dir.path(),
		"other.yaml",
		"services:\n  inc:\n    image: x\n",
	);
	let text = parse_files_with_env_files(&[dir.path().join("compose.yaml")], &[])
		.unwrap_err()
		.to_string();
	assert!(!text.contains('\u{1b}'), "{text:?}");
}
