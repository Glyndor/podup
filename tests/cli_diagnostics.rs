//! CLI-level checks for diagnostic surfacing: forward-compat warnings must be
//! visible with no `RUST_LOG` set, must go to stderr, and must carry the
//! `podup:` program prefix — while stdout stays a clean YAML pipe for `config`.

use std::fs;
use std::process::Command;

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Write a compose file with an unsupported key into a fresh temp dir and
/// return its path. The unknown key drives the forward-compat diagnostic.
fn compose_with_unknown_key() -> std::path::PathBuf {
	let dir = std::env::temp_dir().join(format!("podup-diag-{}", std::process::id()));
	fs::create_dir_all(&dir).expect("create temp dir");
	let path = dir.join("compose.yaml");
	fs::write(
		&path,
		"services:\n  web:\n    image: nginx:1.27\n    bogus_field: oops\n",
	)
	.expect("write compose file");
	path
}

#[test]
fn config_warns_on_stderr_with_clean_stdout_and_no_rust_log() {
	let file = compose_with_unknown_key();
	let output = Command::new(bin())
		.arg("-f")
		.arg(&file)
		.arg("config")
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup config");

	assert!(output.status.success(), "config exits cleanly");

	let stdout = String::from_utf8_lossy(&output.stdout);
	let stderr = String::from_utf8_lossy(&output.stderr);

	// The diagnostic fires by default (no RUST_LOG) and lands on stderr with
	// the unified `podup: warning:` prefix.
	assert!(
		stderr.contains("podup: warning:"),
		"warning prefixed and on stderr; got stderr:\n{stderr}"
	);
	assert!(
		stderr.contains("bogus_field"),
		"warning names the unknown key; got stderr:\n{stderr}"
	);

	// stdout is the resolved YAML only — no diagnostics interleaved.
	assert!(stdout.contains("services:"), "stdout carries the YAML");
	assert!(
		!stdout.contains("warning"),
		"stdout stays a clean pipe; got stdout:\n{stdout}"
	);

	let _ = fs::remove_dir_all(file.parent().unwrap());
}

#[test]
fn completions_emit_a_script_to_stdout() {
	let output = Command::new(bin())
		.args(["completions", "bash"])
		.output()
		.expect("run podup completions bash");
	assert!(output.status.success(), "completions exits cleanly");
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(
		stdout.contains("_podup") || stdout.contains("complete"),
		"emits a bash completion script; got:\n{}",
		&stdout[..stdout.len().min(200)]
	);
}
