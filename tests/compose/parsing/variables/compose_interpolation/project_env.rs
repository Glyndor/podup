//! The variables of a project come from the project directory, not from the
//! directory of each file.
//!
//! The compose specification reads the `.env` of the project: with several
//! `-f` files in different directories, the variables come from the directory
//! of the first file (or from the one `--project-directory` names), and every
//! file is interpolated with them.

use podup::compose::parse_files_with_env_files_interp_in;
use std::fs;
use tempfile::tempdir;

/// A `.env` next to the first file fills the variables of a file that lives
/// somewhere else.
#[test]
fn the_dotenv_comes_from_the_first_file_directory() {
	let dir = tempdir().unwrap();
	let ops = dir.path().join("ops");
	fs::create_dir(&ops).unwrap();
	fs::write(dir.path().join(".env"), "DB_PASS=from-root\n").unwrap();
	fs::write(
		dir.path().join("compose.yml"),
		"services:\n  app:\n    image: nginx\n",
	)
	.unwrap();
	fs::write(
		ops.join("prod.yml"),
		"services:\n  app:\n    environment:\n      DB_PASS: ${DB_PASS}\n",
	)
	.unwrap();

	let file = parse_files_with_env_files_interp_in(
		&[dir.path().join("compose.yml"), ops.join("prod.yml")],
		&[],
		true,
		None,
	)
	.unwrap();
	let env = file.services["app"].environment.to_map();
	assert_eq!(
		env.get("DB_PASS").and_then(|v| v.clone()).as_deref(),
		Some("from-root"),
		"the .env of the project was not used: {env:?}"
	);
}

/// The directory `--project-directory` names is the project's, whatever the
/// first file's directory is.
#[test]
fn an_explicit_project_directory_wins() {
	let dir = tempdir().unwrap();
	let project = dir.path().join("project");
	let ops = dir.path().join("ops");
	fs::create_dir_all(&project).unwrap();
	fs::create_dir(&ops).unwrap();
	fs::write(project.join(".env"), "DB_PASS=from-project\n").unwrap();
	fs::write(
		ops.join("prod.yml"),
		"services:\n  app:\n    environment:\n      DB_PASS: ${DB_PASS}\n",
	)
	.unwrap();

	let file =
		parse_files_with_env_files_interp_in(&[ops.join("prod.yml")], &[], true, Some(&project))
			.unwrap();
	let env = file.services["app"].environment.to_map();
	assert_eq!(
		env.get("DB_PASS").and_then(|v| v.clone()).as_deref(),
		Some("from-project"),
		"the .env of the project directory was not used: {env:?}"
	);
}

/// An `--env-file` is read once, from the project directory, not once per
/// file from the directory of each.
#[test]
fn an_env_file_is_read_from_the_project_directory() {
	let dir = tempdir().unwrap();
	let ops = dir.path().join("ops");
	fs::create_dir(&ops).unwrap();
	fs::write(dir.path().join("vars.env"), "DB_PASS=from-file\n").unwrap();
	fs::write(
		dir.path().join("compose.yml"),
		"services:\n  app:\n    image: nginx\n",
	)
	.unwrap();
	fs::write(
		ops.join("prod.yml"),
		"services:\n  app:\n    environment:\n      DB_PASS: ${DB_PASS}\n",
	)
	.unwrap();

	let file = parse_files_with_env_files_interp_in(
		&[dir.path().join("compose.yml"), ops.join("prod.yml")],
		&["vars.env".to_string()],
		true,
		None,
	)
	.unwrap();
	let env = file.services["app"].environment.to_map();
	assert_eq!(
		env.get("DB_PASS").and_then(|v| v.clone()).as_deref(),
		Some("from-file"),
		"the env file was not read from the project directory: {env:?}"
	);
}
