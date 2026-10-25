//! A service positional does not swallow the flags after it, and the
//! positionals other commands take still parse.
//!
//! Split out from `tests/cli/surface/cli_flags.rs`; the helpers (`run_offline`,
//! `is_clap_usage_error`) are inherited via `use super::*`.

use super::*;

#[test]
fn flags_after_service_positional_parse() {
	// Each of these used to capture the trailing flag as a phantom service.
	for args in [
		&["pull", "web", "--ignore-pull-failures"][..],
		&["push", "web", "--ignore-push-failures"][..],
		&["kill", "web", "-s", "SIGTERM"][..],
		&["wait", "web", "--help"][..],
		&["volumes", "web", "--quiet"][..],
	] {
		let out = run_offline(args);
		assert!(
			!is_clap_usage_error(&out),
			"`{args:?}` should parse interspersed flags; got:\n{}",
			String::from_utf8_lossy(&out.stderr)
		);
	}
}

#[test]
fn images_and_export_positionals_parse() {
	assert!(!is_clap_usage_error(&run_offline(&["images", "web"])));
	assert!(!is_clap_usage_error(&run_offline(&[
		"export", "web", "-o", "-"
	])));
}
