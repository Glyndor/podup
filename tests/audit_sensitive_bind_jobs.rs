//! A job's bind sources are judged like a service's: folded `..` and relative
//! paths resolved against the project directory.

#![cfg(unix)]

use std::process::Command;

fn audit_json(dir: &std::path::Path, volume: &str) -> String {
	let file = dir.join("c.yaml");
	std::fs::write(
		&file,
		format!(
			"services:\n  web:\n    image: alpine:3.20\njobs:\n  nightly:\n    image: alpine:3.20\n    command: [echo, hi]\n    triggers:\n      manual: true\n    volumes:\n      - \"{volume}\"\n"
		),
	)
	.unwrap();
	let out = Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["--socket", "unix:///nonexistent/podup-audit-jobs.sock"])
		.args(["-f", file.to_str().unwrap(), "audit", "--format", "json"])
		.env_remove("PODMAN_SOCKET")
		.output()
		.expect("run podup");
	String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_job_bind_with_dot_dot_that_lands_on_a_sensitive_path_is_flagged() {
	let dir = tempfile::tempdir().unwrap();
	let json = audit_json(dir.path(), "/tmp/../etc:/m");
	assert!(json.contains("sensitive_bind_mount"), "{json}");
	assert!(json.contains("nightly"), "{json}");
}

#[test]
fn a_job_relative_bind_that_resolves_to_the_host_root_is_flagged() {
	let dir = tempfile::tempdir().unwrap();
	let deep = dir.path().join("a/b");
	std::fs::create_dir_all(&deep).unwrap();
	let up = vec![".."; deep.components().count() - 1].join("/");
	let json = audit_json(&deep, &format!("{up}:/host"));
	assert!(json.contains("sensitive_bind_mount"), "{json}");
}
