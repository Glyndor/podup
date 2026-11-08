//! An unusable memory amount stops `config` and `up`, without a Podman daemon.

use std::process::{Command, Output};

fn run(subcommand: &[&str], mem_limit: &str) -> Output {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("compose.yml"),
		format!("services:\n  web:\n    image: alpine\n    mem_limit: {mem_limit}\n"),
	)
	.unwrap();
	Command::new(env!("CARGO_BIN_EXE_podup"))
		.current_dir(dir.path())
		.env_remove("COMPOSE_PROJECT_NAME")
		.args(["--ansi", "never", "-f", "compose.yml", "--socket"])
		.arg(dir.path().join("absent.sock"))
		.args(subcommand)
		.output()
		.unwrap()
}

fn assert_rejected(subcommand: &[&str], mem_limit: &str, shown: &str) {
	let out = run(subcommand, mem_limit);
	assert_eq!(out.status.code(), Some(1), "{subcommand:?}");
	assert_eq!(
		String::from_utf8(out.stderr).unwrap(),
		format!("podup: error: service 'web': mem_limit {shown} is not a valid memory amount\n")
	);
}

#[test]
fn config_rejects_an_impossible_mem_limit() {
	assert_rejected(&["config"], "\"1e100\"", "\"1e100\"");
}

#[test]
fn config_rejects_garbage_and_negative_amounts() {
	assert_rejected(&["config"], "lots", "\"lots\"");
	assert_rejected(&["config"], "-5m", "\"-5m\"");
}

#[test]
fn up_rejects_an_impossible_mem_limit_before_touching_podman() {
	assert_rejected(&["up", "-d"], "\"1e100\"", "\"1e100\"");
}

#[test]
fn config_still_accepts_a_valid_mem_limit() {
	for value in ["512m", "1.5g", "1073741824"] {
		let out = run(&["config"], value);
		assert_eq!(out.status.code(), Some(0), "{value}");
	}
}
