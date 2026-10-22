//! Every flag the command line accepts is named in the help the operator
//! reads.
//!
//! Split out from `tests/cli_flags.rs`; the helpers (`help_of`, `bin`) are
//! inherited via `use super::*`.

use super::*;

#[test]
fn create_exposes_pull_and_no_deps() {
	let h = help_of("create");
	assert!(h.contains("--no-deps"), "create missing --no-deps:\n{h}");
	assert!(h.contains("--pull"), "create missing --pull:\n{h}");
}

#[test]
fn new_command_flags_appear_in_help() {
	assert!(help_of("ls").contains("--filter"));
	let push = help_of("push");
	assert!(push.contains("--quiet"));
	let build = help_of("build");
	assert!(build.contains("--push") && build.contains("--progress"));
	assert!(help_of("run").contains("--label"));
	let events = help_of("events");
	assert!(
		events.contains("--since") && events.contains("--until") && events.contains("--filter")
	);
	assert!(help_of("attach").contains("--index"));
	let logs = help_of("logs");
	assert!(logs.contains("--no-color") && logs.contains("--no-log-prefix"));
	let config = help_of("config");
	for flag in [
		"--volumes",
		"--images",
		"--profiles",
		"--hash",
		"--no-normalize",
	] {
		assert!(config.contains(flag), "config missing {flag}:\n{config}");
	}
}

#[test]
fn ansi_help_text_clarifies_no_color_override() {
	let out = Command::new(bin()).arg("--help").output().unwrap();
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		stdout.contains("overrides"),
		"--ansi help should clarify that `always` overrides NO_COLOR:\n{stdout}"
	);
}
