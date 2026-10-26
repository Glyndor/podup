//! Unset interpolation variables warn exactly once each, on `config` and on
//! `audit` alike, and never once per reference.
//!
//! Split out from `tests/cli/surface/cli_diagnostics.rs`; the helpers (`bin`,
//! `compose_file`) are inherited via `use super::*`.

use super::*;

#[test]
fn config_warns_on_unset_interpolation_variable() {
	// An unset `${VAR}` interpolates to an empty string but must warn on stderr
	// (matching docker compose) so a config typo does not pass silently.
	let (_dir, compose) = compose_file(
		"docker-compose.yml",
		"services:\n  web:\n    image: ${PODUP_UNSET_IMAGE}\n",
	);
	let c = compose.to_str().unwrap();

	let out = Command::new(bin())
		.args(["-f", c, "config"])
		.env_remove("PODUP_UNSET_IMAGE")
		.output()
		.unwrap();
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains("PODUP_UNSET_IMAGE") && stderr.contains("not set"),
		"expected an unset-variable warning on stderr, got: {stderr:?}"
	);
}

/// Count the stderr lines that name the given unset variable. Used by the
/// count-based diagnostics tests below. A `contains` assertion would pass at
/// 2 today and at 10 tomorrow, which is precisely why #1838 was missed.
fn count_warnings(stderr: &str, var: &str) -> usize {
	let needle = format!("The {var} variable is not set.");
	stderr.lines().filter(|line| line.contains(&needle)).count()
}

/// Pin the exact warning count after the #1838 fix. The compose document is
/// interpolated once during the parse and once during the raw nested-key
/// diagnostic; before the fix both passes emitted the warning, so one
/// `${X}` once fired two stderr lines and `audit` (which goes through the
/// same parse) fired the same two. With the fix the diagnostic pass is
/// silent, so the parse pass alone determines the count.
fn assert_count(command: &str, yaml: &str, var: &str, expected: usize) {
	let (_dir, file) = compose_file("docker-compose.yml", yaml);
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), command])
		.env_remove(var)
		.output()
		.expect("run podup");
	let stderr = String::from_utf8_lossy(&out.stderr);
	let got = count_warnings(&stderr, var);
	assert_eq!(
		got, expected,
		"{command}: expected {expected} warning(s) for {var}, got {got}; stderr:\n{stderr}"
	);
}

/// One `${X}` referenced once: exactly one warning on `config` and on
/// `audit`. The previous behaviour was two on `config`, four on `audit`
/// (#1838).
#[test]
fn unset_variable_once_warns_once_on_config_and_audit() {
	let yaml = "services:\n  web:\n    image: ${PODUP_WARN_COUNT_X}\n";
	assert_count("config", yaml, "PODUP_WARN_COUNT_X", 1);
	assert_count("audit", yaml, "PODUP_WARN_COUNT_X", 1);
}

/// Two different unset variables, each referenced once: exactly two
/// warnings, on `config` and on `audit`. The previous behaviour was four
/// on `config`, eight on `audit` (#1838).
#[test]
fn two_unset_variables_warn_twice_on_config_and_audit() {
	let yaml = "services:\n  web:\n    image: ${PODUP_WARN_COUNT_A}\n    environment:\n      X: ${PODUP_WARN_COUNT_B}\n";
	assert_count("config", yaml, "PODUP_WARN_COUNT_A", 1);
	assert_count("config", yaml, "PODUP_WARN_COUNT_B", 1);
	assert_count("audit", yaml, "PODUP_WARN_COUNT_A", 1);
	assert_count("audit", yaml, "PODUP_WARN_COUNT_B", 1);
}

/// The same unset variable referenced three times: warn once per call. A
/// missing `FOO` is one piece of information regardless of how many
/// scalars reference it; docker compose reports it once, the per-call
/// `HashSet<String>` in [`substitute_depth`] enforces the same shape.
/// The previous behaviour was six on `config`, twelve on `audit`
/// (#1838).
#[test]
fn same_unset_variable_three_times_warns_once_on_config_and_audit() {
	let yaml = "services:\n  web:\n    image: ${PODUP_WARN_COUNT_REP}\n    environment:\n      A: ${PODUP_WARN_COUNT_REP}\n      B: ${PODUP_WARN_COUNT_REP}\n";
	assert_count("config", yaml, "PODUP_WARN_COUNT_REP", 1);
	assert_count("audit", yaml, "PODUP_WARN_COUNT_REP", 1);
}

/// Bare `$X` references (no braces) go through the second emitter at
/// `internal/parsing/substitute/mod.rs`, distinct from `${X}` in `parse.rs`. Both
/// emitters feed through the same dedup + silence gate, so a bare
/// reference behaves identically: one warning per unique variable per
/// call, on both `config` and `audit`.
#[test]
fn bare_unset_variable_once_warns_once_on_config_and_audit() {
	let yaml = "services:\n  web:\n    image: $PODUP_WARN_COUNT_BARE\n";
	assert_count("config", yaml, "PODUP_WARN_COUNT_BARE", 1);
	assert_count("audit", yaml, "PODUP_WARN_COUNT_BARE", 1);
}

/// Property over the surface: for any number N of distinct unset variables
/// in a single parse, the **total** warning count on `config` is N and the
/// **total** count on `audit` is N. The example-based tests above check
/// individual names; this one checks the invariant itself, so a future
/// emitter that re-fires for every reference is caught the moment a file
/// carries enough variables to make the duplication observable.
///
/// Before the #1838 fix a file with one variable fired two warnings on
/// `config` and four on `audit`. A `contains` assertion passed at 2 and
/// would have passed at 10; the only way to catch the shape of the bug is
/// to count, and the only way to catch its next instance is to vary the
/// input.
#[test]
fn unset_variable_count_is_exactly_one_per_variable_on_config_and_audit() {
	// Eight distinct unset variables, each referenced once. The chosen count
	// is the smallest one that makes the multiplier of the previous bug
	// (2x on config, 4x on audit) conspicuous against the expected total of
	// 8: with the bug, the totals would be 16 and 32, both far enough from
	// 8 that a counted assertion cannot pass by accident.
	let vars: Vec<String> = (0..8).map(|i| format!("PODUP_PROP_COUNT_{i}")).collect();
	let mut yaml = String::from("services:\n  web:\n    image: alpine\n    environment:\n");
	for v in &vars {
		let line = format!("      VAR_{v}: ${{{v}}}\n");
		let _ = std::fmt::Write::write_str(&mut yaml, &line);
	}
	let (_dir, file) = compose_file("docker-compose.yml", &yaml);

	for command in ["config", "audit"] {
		let out = Command::new(bin())
			.args(["-f", file.to_str().unwrap(), command])
			.env_remove("PODUP_PROP_COUNT_0")
			.env_remove("PODUP_PROP_COUNT_1")
			.env_remove("PODUP_PROP_COUNT_2")
			.env_remove("PODUP_PROP_COUNT_3")
			.env_remove("PODUP_PROP_COUNT_4")
			.env_remove("PODUP_PROP_COUNT_5")
			.env_remove("PODUP_PROP_COUNT_6")
			.env_remove("PODUP_PROP_COUNT_7")
			.output()
			.expect("run podup");
		let stderr = String::from_utf8_lossy(&out.stderr);
		let total = stderr
			.lines()
			.filter(|l| l.contains("variable is not set"))
			.count();
		assert_eq!(
			total,
			vars.len(),
			"{command}: expected exactly {} unset-variable warnings (one per variable), got {total}; stderr:\n{stderr}",
			vars.len()
		);
	}
}
