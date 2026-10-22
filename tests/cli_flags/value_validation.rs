//! Flag values are validated at parse time: a value outside the accepted set
//! is a parse error, and a bare boolean stays bare for the scripts written
//! against docker compose.
//!
//! Split out from `tests/cli_flags.rs`; the helpers (`run_offline`,
//! `is_clap_value_error`, `is_clap_usage_error`) are inherited via
//! `use super::*`.

use super::*;

#[test]
fn invalid_pull_policy_is_rejected() {
	for args in [
		&["pull", "--policy", "bogus"][..],
		&["up", "--pull", "nope"][..],
	] {
		let out = run_offline(args);
		assert!(
			is_clap_value_error(&out),
			"`{args:?}` should be a parse error; got:\n{}",
			String::from_utf8_lossy(&out.stderr)
		);
	}
	// A valid value parses.
	assert!(!is_clap_value_error(&run_offline(&[
		"pull", "--policy", "always", "web"
	])));
}

#[test]
fn negative_timeout_is_rejected_consistently() {
	for args in [
		&["down", "--timeout=-5"][..],
		&["down", "-t", "-5"][..],
		&["stop", "-t", "-1"][..],
	] {
		assert!(
			is_clap_value_error(&run_offline(args)),
			"`{args:?}` should be a clear range error"
		);
	}
	// Zero and positive are fine.
	assert!(!is_clap_value_error(&run_offline(&["down", "--timeout=0"])));
}

/// `--sig-proxy` must accept docker's bare spelling.
///
/// It lives in the "accepted for compatibility" set, whose entire promise is
/// that a script written against docker compose runs unchanged. Docker declares
/// it as a bare boolean; podup declared it as one that demands a value, so
/// `docker compose attach --sig-proxy web` came back as a usage error and exit
/// 2, a no-op flag rejecting the command it was supposed to tolerate.
///
/// Asserted as "not a clap error": these never reach Podman, so the run fails
/// later on the missing compose file, which is exactly the point: parsing got
/// past the flag.
#[test]
fn sig_proxy_accepts_dockers_bare_form_and_an_explicit_value() {
	for args in [
		vec!["attach", "web", "--sig-proxy"],
		vec!["attach", "web", "--sig-proxy=true"],
		vec!["attach", "web", "--sig-proxy=false"],
	] {
		let out = run_offline(&args);
		assert!(
			!is_clap_usage_error(&out) && !is_clap_value_error(&out),
			"{args:?} must parse; got {}",
			String::from_utf8_lossy(&out.stderr)
		);
	}
}
