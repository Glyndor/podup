//! Forward-compat diagnostics: an unsupported key surfaces with no `RUST_LOG`
//! set, on stderr, with the `podup:` prefix, and never pollutes stdout.
//!
//! Split out from `tests/cli_diagnostics.rs`; the helpers (`bin`,
//! `compose_file`) are inherited via `use super::*`.

use super::*;

/// A compose file with an unsupported key, which drives the forward-compat
/// diagnostic.
fn compose_with_unknown_key() -> (TempDir, PathBuf) {
	compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: nginx:1.27\n    bogus_field: oops\n",
	)
}

#[test]
fn config_warns_on_stderr_with_clean_stdout_and_no_rust_log() {
	let (_dir, file) = compose_with_unknown_key();
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

	// stdout is the resolved YAML only, with no diagnostics interleaved.
	assert!(stdout.contains("services:"), "stdout carries the YAML");
	assert!(
		!stdout.contains("warning"),
		"stdout stays a clean pipe; got stdout:\n{stdout}"
	);
}
