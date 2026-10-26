//! The `completions` subcommand writes its script to stdout.
//!
//! Split out from `tests/cli/surface/cli_diagnostics.rs`; the helper `bin` is inherited
//! via `use super::*`.

use super::*;

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
