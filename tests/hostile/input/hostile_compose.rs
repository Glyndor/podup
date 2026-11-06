//! Compose files written to hurt the loader, seen from the command line.
//!
//! `podup config` reads a compose file through the loader every command uses,
//! so a file that makes it hang, exhaust memory, crash or leak the content of
//! a file it was never meant to read would do the same in `up`. Each case
//! builds one hostile file and checks the same four things: the process ends
//! inside a time budget, it is not killed by a signal and does not panic, it
//! stays under a resident memory ceiling, and it does not print what it should
//! not have read. Where a guard exists to refuse the file, the exit code must
//! also be non-zero.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

#[path = "hostile_compose/caps.rs"]
mod caps;
#[path = "../support/file_process.rs"]
mod file_process;
use file_process::{isolated, run, Outcome};

/// Resident ceiling for any run. The honest cases stay far below it; a loader
/// that reserved gigabytes before refusing would not.
const RSS_CEILING_KIB: u64 = 256 * 1024;

/// A socket path that cannot be connected to, so nothing here reaches Podman.
const DEAD_SOCKET: &str = "unix:///nonexistent/podup-hostile-compose-test.sock";

/// Write `contents` as `name` in `dir`.
fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
	let path = dir.join(name);
	fs::write(&path, contents).expect("write file");
	path
}

/// Run `podup -f <file> config` with an isolated home, killing it at the
/// deadline `file_process::run` sets.
///
/// `code` is `None` when the process was killed, by the deadline or by a
/// signal of its own, and the checks below treat both as a failure of podup.
fn config(file: &Path) -> Outcome {
	let dir = file.parent().expect("a compose file has a directory");
	let mut cmd = isolated(dir);
	cmd.args(["--socket", DEAD_SOCKET, "-f"])
		.arg(file)
		.arg("config");
	run(cmd)
}

/// The process ended by itself, with an exit code, did not panic, and stayed
/// under the resident ceiling.
fn assert_survived(what: &str, outcome: &Outcome) {
	assert!(
		outcome.code.is_some(),
		"{what}: podup was killed (a hang past the deadline or a signal)"
	);
	assert!(
		!outcome.text.contains("panicked at"),
		"{what}: podup panicked:\n{}",
		outcome.text
	);
	assert!(
		outcome.rss_kib < RSS_CEILING_KIB,
		"{what}: {} KiB resident, ceiling {RSS_CEILING_KIB} KiB",
		outcome.rss_kib
	);
}

/// [`assert_survived`] for a guard that must refuse the file: a non-zero exit.
fn assert_refused(what: &str, outcome: &Outcome) {
	assert_survived(what, outcome);
	assert_ne!(outcome.code, Some(0), "{what}: the file was accepted");
}

fn compose_dir() -> TempDir {
	tempfile::tempdir().expect("create temp dir")
}

#[test]
fn an_alias_bomb_does_not_exhaust_the_loader() {
	// Nine levels, each aliasing the one below ten times: a billion leaves if
	// anything expands the aliases instead of refusing them.
	let mut yaml = String::from("x-l0: &l0 [lol, lol, lol, lol, lol, lol, lol, lol, lol, lol]\n");
	for level in 1..9 {
		let below = level - 1;
		let refs = vec![format!("*l{below}"); 10].join(", ");
		yaml.push_str(&format!("x-l{level}: &l{level} [{refs}]\n"));
	}
	yaml.push_str("services:\n  web:\n    image: alpine\n    command: *l8\n");
	let dir = compose_dir();
	let outcome = config(&write(dir.path(), "compose.yaml", &yaml));
	assert_refused("alias bomb", &outcome);
}

#[test]
fn a_very_deeply_nested_file_is_refused_not_a_stack_overflow() {
	let depth = 200_000;
	let yaml = format!(
		"services:\n  web:\n    image: alpine\n    command: {}{}\n",
		"[".repeat(depth),
		"]".repeat(depth)
	);
	let dir = compose_dir();
	let outcome = config(&write(dir.path(), "compose.yaml", &yaml));
	assert_refused("deep nesting", &outcome);
}

#[test]
fn a_huge_scalar_ends_in_the_budget() {
	let yaml = format!(
		"services:\n  web:\n    image: alpine\n    command: [\"{}\"]\n",
		"a".repeat(16 * 1024 * 1024)
	);
	let dir = compose_dir();
	let outcome = config(&write(dir.path(), "compose.yaml", &yaml));
	assert_refused("16 MiB scalar", &outcome);
}

#[test]
fn an_extends_cycle_is_refused() {
	let dir = compose_dir();
	write(
		dir.path(),
		"a.yaml",
		"services:\n  web:\n    extends: {file: b.yaml, service: web}\n",
	);
	write(
		dir.path(),
		"b.yaml",
		"services:\n  web:\n    extends: {file: a.yaml, service: web}\n",
	);
	let outcome = config(&dir.path().join("a.yaml"));
	assert_refused("extends cycle", &outcome);
}

#[test]
fn an_extends_chain_past_the_depth_cap_is_refused() {
	let dir = compose_dir();
	for index in 0..40 {
		let next = index + 1;
		write(
			dir.path(),
			&format!("f{index}.yaml"),
			&format!("services:\n  web:\n    extends: {{file: f{next}.yaml, service: web}}\n"),
		);
	}
	write(
		dir.path(),
		"f40.yaml",
		"services:\n  web:\n    image: alpine\n",
	);
	let outcome = config(&dir.path().join("f0.yaml"));
	assert_refused("long extends chain", &outcome);
}

#[test]
fn an_include_loop_ends() {
	let dir = compose_dir();
	write(
		dir.path(),
		"a.yaml",
		"include: [b.yaml]\nservices:\n  a:\n    image: alpine\n",
	);
	write(
		dir.path(),
		"b.yaml",
		"include: [a.yaml]\nservices:\n  b:\n    image: alpine\n",
	);
	let outcome = config(&dir.path().join("a.yaml"));
	// The loader refuses a circular include, so it cannot run away.
	assert_refused("include loop", &outcome);
}

#[test]
fn a_self_referencing_interpolation_ends() {
	let dir = compose_dir();
	let nested = format!("{}x{}", "${A:-".repeat(2_000), "}".repeat(2_000));
	let yaml = format!(
		"services:\n  web:\n    image: alpine\n    environment:\n      A: '${{A}}'\n      B: '{nested}'\n"
	);
	let outcome = config(&write(dir.path(), "compose.yaml", &yaml));
	assert_survived("self-referencing interpolation", &outcome);
}

#[test]
fn an_extends_file_outside_the_project_is_not_echoed() {
	// The target is a file that exists and holds something that is not YAML
	// for a compose file. Whatever podup says about it, it must not print
	// the content back.
	let dir = compose_dir();
	let secret = write(
		dir.path(),
		"secret.txt",
		"TOP-SECRET-MARKER-9f3a: do-not-print\n",
	);
	let project = dir.path().join("project");
	fs::create_dir(&project).unwrap();
	let compose = write(
		&project,
		"compose.yaml",
		&format!(
			"services:\n  web:\n    extends: {{file: {}, service: web}}\n",
			secret.display()
		),
	);
	let outcome = config(&compose);
	assert_survived("extends of a foreign file", &outcome);
	assert!(
		!outcome.text.contains("TOP-SECRET-MARKER-9f3a"),
		"the content of the file leaked:\n{}",
		outcome.text
	);
}

#[cfg(unix)]
#[test]
fn an_env_file_that_is_a_symlink_to_a_secret_is_not_echoed() {
	let dir = compose_dir();
	let secret = write(
		dir.path(),
		"secret.txt",
		"TOP-SECRET-MARKER-7c1e\nnot a key value line\n",
	);
	let project = dir.path().join("project");
	fs::create_dir(&project).unwrap();
	std::os::unix::fs::symlink(&secret, project.join("app.env")).unwrap();
	let compose = write(
		&project,
		"compose.yaml",
		"services:\n  web:\n    image: alpine\n    env_file: [app.env]\n",
	);
	let outcome = config(&compose);
	assert_survived("env_file symlink", &outcome);
	assert!(
		!outcome.text.contains("TOP-SECRET-MARKER-7c1e"),
		"the content of the file leaked:\n{}",
		outcome.text
	);
}

#[test]
fn control_characters_in_a_service_name_do_not_reach_the_terminal() {
	let dir = compose_dir();
	let outcome = config(&write(
		dir.path(),
		"compose.yaml",
		"services:\n  \"web\\x1b[2J\\x07\":\n    image: alpine\n",
	));
	assert_survived("control characters in a name", &outcome);
	assert!(
		!outcome.text.contains('\u{1b}') && !outcome.text.contains('\u{7}'),
		"raw control characters reached the output:\n{:?}",
		outcome.text
	);
}
