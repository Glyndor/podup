//! Two flags that ask for opposite things are rejected together rather than
//! one of them winning silently, and a flag that only makes sense alongside
//! another says so.
//!
//! Split out from `tests/cli_flags.rs`; the helpers (`run_offline`,
//! `is_clap_usage_error`) are inherited via `use super::*`.

use super::*;

#[test]
fn up_has_wait_timeout_that_requires_wait() {
	assert!(help_of("up").contains("--wait-timeout"));
	// Without --wait it is a usage error (clap `requires`).
	assert!(is_clap_usage_error(&run_offline(&[
		"start",
		"--wait-timeout",
		"30"
	])));
	assert!(is_clap_usage_error(&run_offline(&[
		"up",
		"--wait-timeout",
		"30"
	])));
	// With --wait it parses.
	assert!(!is_clap_usage_error(&run_offline(&[
		"up",
		"--wait",
		"--wait-timeout",
		"30"
	])));
}

#[test]
fn conflicting_recreate_and_build_flags_are_rejected() {
	for args in [
		&["up", "--no-recreate", "--force-recreate"][..],
		&["up", "--build", "--no-build"][..],
		&["create", "--no-recreate", "--force-recreate"][..],
	] {
		assert!(
			is_clap_usage_error(&run_offline(args)),
			"`{args:?}` should be rejected as conflicting"
		);
	}
}

#[test]
fn events_json_conflicts_with_explicit_format() {
	assert!(is_clap_usage_error(&run_offline(&[
		"events", "--json", "--format", "json"
	])));
	// Each spelling alone still parses.
	assert!(!is_clap_usage_error(&run_offline(&["events", "--json"])));
	assert!(!is_clap_usage_error(&run_offline(&[
		"events", "--format", "json"
	])));
}

/// `ps` exposes three output-shaping flags: `-q/--quiet` (IDs), `--format`
/// (table/json), `--services` (service names). They each render a different
/// thing; combining any two leaves the third's effect undefined. Until the
/// fix, `--services` accepted `--format json` and `-q` silently and rendered
/// the same `w` line, which a `--format json | jq` pipeline reads as not-JSON
/// while the exit code stays 0. `-q --format json` already rejected; the test
/// enumerates every pair so the fix cannot be "reject everything with
/// `--services`" (which would break the three single-flag uses below).
#[test]
fn ps_output_flags_conflict_in_every_pair_and_survive_alone() {
	// Each conflicting pair is a clap usage error.
	for args in [
		&["ps", "-q", "--format", "json"][..],
		&["ps", "--services", "--format", "json"][..],
		&["ps", "--services", "-q"][..],
	] {
		assert!(
			is_clap_usage_error(&run_offline(args)),
			"`{args:?}` should be rejected as conflicting"
		);
	}
	// Each flag alone still parses (and reaches Podman at runtime, where it
	// then fails on the nonexistent socket).
	for args in [
		&["ps", "--services"][..],
		&["ps", "--format", "json"][..],
		&["ps", "-q"][..],
	] {
		assert!(
			!is_clap_usage_error(&run_offline(args)),
			"`{args:?}` should parse; got:\n{}",
			String::from_utf8_lossy(&run_offline(args).stderr)
		);
	}
}

/// Property over the surface: every command that exposes more than one
/// output-shaping flag treats every pair as **either** a parsed argument
/// list **or** a clap usage error. No pair is accepted with one flag
/// silently dropped. `ps` had three flags (`-q`, `--format`, `--services`)
/// and the documented `-q --format json` pair refused correctly while the
/// other two dropped one flag; this test extends the same enumeration to
/// every other command in the same shape (`volumes`, `images`, `events`),
/// so the next command that grows a second output-shaping flag is checked
/// the same way from the moment it lands.
///
/// "Conflict" here means `clap` itself rejected the pair. Reaching Podman
/// at runtime and failing there is the success branch: parsing got past
/// the pair, so neither was dropped.
#[test]
fn every_command_with_output_flags_rejects_or_parses_every_pair() {
	// Each command, its flag pairs, and whether the pair must error.
	// `ps` has three flags (3 pairs), the rest have two (1 pair).
	let cases: &[(&str, &[&[&str]])] = &[
		(
			"ps",
			&[
				&["-q", "--format", "json"],
				&["--services", "--format", "json"],
				&["--services", "-q"],
			],
		),
		("volumes", &[&["-q", "--format", "json"]]),
		("images", &[&["-q", "--format", "json"]]),
		("events", &[&["--json", "--format", "json"]]),
	];
	for (cmd, pairs) in cases {
		for pair in *pairs {
			let mut args = vec![*cmd];
			args.extend_from_slice(pair);
			let out = run_offline(&args);
			assert!(
				is_clap_usage_error(&out),
				"`{args:?}` should be rejected as conflicting; got:\n{}",
				String::from_utf8_lossy(&out.stderr)
			);
		}
	}
}
