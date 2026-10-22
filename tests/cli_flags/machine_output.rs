//! Machine-readable output carries no escape codes, even when colour is
//! forced.
//!
//! Split out from `tests/cli_flags.rs`; the helper `bin` is inherited via
//! `use super::*`.

use super::*;

/// Machine-readable output must never carry colour, even when colour is
/// forced. A script parsing `--format json` or `-q` reads stdout, so any
/// escape landing there is data corruption, not decoration.
///
/// Forced with `--ansi always` on purpose: the auto-detection would suppress
/// colour in the test harness anyway, so a weaker version of this test would
/// pass without proving anything. Only stdout is checked: a connection
/// failure prints its `error:` banner to stderr, which is a human diagnostic
/// and is allowed to be coloured like any other podup error; the guarantee
/// under test is about the machine-parseable stream, not every byte podup
/// writes.
#[test]
fn machine_output_carries_no_escapes_even_with_colour_forced() {
	let dir = std::env::temp_dir().join(format!("podup-cflags-machine-{}", std::process::id()));
	fs::create_dir_all(&dir).unwrap();
	let compose = dir.join("compose.yaml");
	fs::write(&compose, "services:\n  web:\n    image: nginx:1.27\n").unwrap();
	let c = compose.to_str().unwrap();

	for args in [
		vec!["-f", c, "--ansi", "always", "config"],
		vec!["-f", c, "--ansi", "always", "ps", "--format", "json"],
		vec!["-f", c, "--ansi", "always", "ps", "-q"],
	] {
		let out = Command::new(bin())
			.args(&args)
			.output()
			.unwrap_or_else(|e| panic!("run podup {args:?}: {e}"));
		let stdout = String::from_utf8_lossy(&out.stdout);
		assert!(
			!stdout.contains('\u{1b}'),
			"{args:?} emitted an escape into machine output: {stdout:?}"
		);
	}

	let _ = fs::remove_dir_all(&dir);
}
