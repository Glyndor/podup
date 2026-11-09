//! Where `config` finds the values of the project environment: bare keys under
//! `environment:`, `--env-file` paths, and a `.env` that is a link. None of it
//! needs a Podman daemon.

use std::path::Path;
use std::process::{Command, Output};

fn podup(cwd: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_podup"))
		.current_dir(cwd)
		.env_remove("COMPOSE_PROJECT_NAME")
		.env_remove("COMPOSE_ENV_FILES")
		.env_remove("BARE_PORT")
		.env_remove("BARE_ABSENT")
		.args(["--ansi", "never", "-p", "x"])
		.args(args)
		.output()
		.unwrap()
}

fn stdout(out: &Output) -> String {
	assert!(
		out.status.success(),
		"{}",
		String::from_utf8_lossy(&out.stderr)
	);
	String::from_utf8(out.stdout.clone()).unwrap()
}

fn compose_with(dir: &Path, body: &str) {
	std::fs::write(
		dir.join("compose.yml"),
		format!("services:\n  a:\n    image: busybox\n{body}"),
	)
	.unwrap();
}

#[test]
fn config_resolves_a_bare_key_from_dotenv_and_leaves_an_absent_one_null() {
	let dir = tempfile::tempdir().unwrap();
	compose_with(
		dir.path(),
		"    environment: [BARE_PORT, BARE_ABSENT, FIXED=a]\n",
	);
	std::fs::write(dir.path().join(".env"), "BARE_PORT=18081\n").unwrap();
	let out = stdout(&podup(dir.path(), &["config"]));
	assert!(out.contains("- BARE_PORT=18081"), "{out}");
	assert!(out.contains("- BARE_ABSENT\n"), "{out}");
	assert!(out.contains("- FIXED=a"), "{out}");
}

#[test]
fn config_resolves_a_bare_key_from_an_env_file() {
	let dir = tempfile::tempdir().unwrap();
	compose_with(dir.path(), "    environment: [BARE_PORT]\n");
	std::fs::write(dir.path().join(".env"), "BARE_PORT=from-dotenv\n").unwrap();
	std::fs::write(dir.path().join("a.env"), "BARE_PORT=from-file\n").unwrap();
	let out = stdout(&podup(dir.path(), &["--env-file", "a.env", "config"]));
	assert!(out.contains("- BARE_PORT=from-file"), "{out}");
}

#[test]
fn the_process_environment_wins_over_dotenv_for_a_bare_key() {
	let dir = tempfile::tempdir().unwrap();
	compose_with(dir.path(), "    environment: [BARE_PORT]\n");
	std::fs::write(dir.path().join(".env"), "BARE_PORT=from-dotenv\n").unwrap();
	let mut cmd = Command::new(env!("CARGO_BIN_EXE_podup"));
	let out = cmd
		.current_dir(dir.path())
		.env("BARE_PORT", "from-shell")
		.args(["--ansi", "never", "-p", "x", "config"])
		.output()
		.unwrap();
	assert!(stdout(&out).contains("- BARE_PORT=from-shell"));
}

#[test]
fn a_relative_env_file_is_read_from_the_working_directory() {
	let dir = tempfile::tempdir().unwrap();
	let envs = dir.path().join("env-files");
	std::fs::create_dir(&envs).unwrap();
	compose_with(dir.path(), "    environment: [\"FOO=${FOO}\"]\n");
	std::fs::write(envs.join("my-env"), "FOO=foo\n").unwrap();
	let out = stdout(&podup(
		&envs,
		&["--env-file", "my-env", "-f", "../compose.yml", "config"],
	));
	assert!(out.contains("- FOO=foo"), "{out}");
}

#[test]
fn a_relative_env_file_is_not_looked_up_next_to_the_compose_file() {
	let dir = tempfile::tempdir().unwrap();
	compose_with(dir.path(), "    environment: [\"FOO=${FOO}\"]\n");
	std::fs::write(dir.path().join("my-env"), "FOO=foo\n").unwrap();
	let sub = dir.path().join("elsewhere");
	std::fs::create_dir(&sub).unwrap();
	let out = podup(
		&sub,
		&["--env-file", "my-env", "-f", "../compose.yml", "config"],
	);
	assert!(!out.status.success());
	assert!(String::from_utf8_lossy(&out.stderr).contains("env file not found"));
}

#[cfg(unix)]
#[test]
fn a_dotenv_that_is_a_link_is_read() {
	let dir = tempfile::tempdir().unwrap();
	compose_with(dir.path(), "    environment: [\"TAG=${LINKED_TAG}\"]\n");
	std::fs::write(dir.path().join("real.env"), "LINKED_TAG=3.19\n").unwrap();
	std::os::unix::fs::symlink("real.env", dir.path().join(".env")).unwrap();
	let out = stdout(&podup(dir.path(), &["config"]));
	assert!(out.contains("- TAG=3.19"), "{out}");
}

#[test]
fn config_resolves_a_null_map_value_like_a_bare_key() {
	let dir = tempfile::tempdir().unwrap();
	compose_with(
		dir.path(),
		"    environment:\n      BARE_PORT:\n      BARE_ABSENT:\n      FIXED: a\n",
	);
	std::fs::write(dir.path().join(".env"), "BARE_PORT=18081\n").unwrap();
	let out = stdout(&podup(dir.path(), &["config"]));
	assert!(out.contains("BARE_PORT: '18081'"), "{out}");
	assert!(out.contains("BARE_ABSENT: null"), "{out}");
	assert!(out.contains("FIXED: a"), "{out}");
	std::fs::write(dir.path().join("a.env"), "BARE_PORT=from-file\n").unwrap();
	let out = stdout(&podup(dir.path(), &["--env-file", "a.env", "config"]));
	assert!(out.contains("BARE_PORT: from-file"), "{out}");
}
