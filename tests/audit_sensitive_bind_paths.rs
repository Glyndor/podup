//! `podup audit` judges a bind source by where it lands: dot-dot components and relative paths.
#![cfg(unix)]

use std::fs;
use std::process::Command;

const DEAD_SOCKET: &str = "unix:///nonexistent/podup-hostile-fs-defects.sock";

fn audit_json(volume: &str) -> String {
	let dir = tempfile::tempdir().expect("tempdir");
	let file = dir.path().join("c.yaml");
	fs::write(
		&file,
		format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - \"{volume}\"\n"),
	)
	.unwrap();
	let out = Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["-f", file.to_str().unwrap(), "audit", "--format", "json"])
		.env("DOCKER_HOST", DEAD_SOCKET)
		.env_remove("PODMAN_SOCKET")
		.output()
		.expect("run podup");
	String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_dot_dot_that_lands_on_a_sensitive_path_is_still_flagged() {
	for source in [
		"/tmp/../etc:/m",
		"/./etc:/m",
		"/usr/../root:/m",
		"/home/../proc:/m",
		"/var/run/../run/docker.sock:/s",
	] {
		assert!(
			audit_json(source).contains("sensitive_bind_mount"),
			"{source} resolves to a sensitive path and was not flagged"
		);
	}
}

#[test]
fn a_relative_source_that_resolves_to_the_host_root_is_flagged() {
	// `config` resolves `../../..` to an absolute path, from a directory three
	// levels below `/`, but the audit reads the string as written.
	let dir = tempfile::tempdir().expect("tempdir");
	let deep = dir.path().join("a/b/c");
	fs::create_dir_all(&deep).unwrap();
	let depth = deep.components().count() - 1;
	let up = vec![".."; depth].join("/");
	let file = deep.join("c.yaml");
	fs::write(
		&file,
		format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - \"{up}:/host\"\n"
		),
	)
	.unwrap();
	let out = Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["-f", file.to_str().unwrap(), "audit", "--format", "json"])
		.env("DOCKER_HOST", DEAD_SOCKET)
		.env_remove("PODMAN_SOCKET")
		.output()
		.expect("run podup");
	assert!(String::from_utf8_lossy(&out.stdout).contains("sensitive_bind_mount"));
}
