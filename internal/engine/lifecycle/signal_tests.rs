use super::{must_wait_after_kill, validate_signal};
use crate::error::ComposeError;

#[test]
fn accepts_common_signal_names() {
	for s in ["SIGKILL", "SIGTERM", "SIGHUP", "SIGINT", "SIGUSR1"] {
		assert!(validate_signal(s).is_ok(), "{s} should be accepted");
	}
}

#[test]
fn accepts_names_without_sig_prefix_case_insensitive() {
	assert!(validate_signal("TERM").is_ok());
	assert!(validate_signal("term").is_ok());
	assert!(validate_signal("Kill").is_ok());
}

#[test]
fn accepts_numeric_signals_in_range() {
	assert!(validate_signal("9").is_ok());
	assert!(validate_signal("15").is_ok());
	assert!(validate_signal("1").is_ok());
	assert!(validate_signal("64").is_ok());
}

#[test]
fn rejects_empty_signal() {
	// The core bug: an empty signal must not be forwarded (it would default
	// to SIGKILL on the libpod side).
	let err = validate_signal("").unwrap_err();
	assert!(matches!(err, ComposeError::InvalidSignal(_)));
	assert!(err.to_string().contains("invalid signal"));
}

#[test]
fn rejects_whitespace_only_signal() {
	assert!(matches!(
		validate_signal("   ").unwrap_err(),
		ComposeError::InvalidSignal(_)
	));
}

#[test]
fn rejects_out_of_range_and_zero_numbers() {
	assert!(matches!(
		validate_signal("0").unwrap_err(),
		ComposeError::InvalidSignal(_)
	));
	assert!(matches!(
		validate_signal("65").unwrap_err(),
		ComposeError::InvalidSignal(_)
	));
	assert!(matches!(
		validate_signal("9999").unwrap_err(),
		ComposeError::InvalidSignal(_)
	));
}

#[test]
fn rejects_unknown_signal_names() {
	assert!(matches!(
		validate_signal("SIGBOGUS").unwrap_err(),
		ComposeError::InvalidSignal(_)
	));
	assert!(matches!(
		validate_signal("not-a-signal").unwrap_err(),
		ComposeError::InvalidSignal(_)
	));
}

#[test]
fn must_wait_after_kill_only_for_kill_or_zero() {
	// SIGKILL/9/KILL/cased and 0 trigger the follow-up wait; every other
	// signal the user is likely to send does not, and the helper pins the
	// boundary so a future "always wait" simplification does not silently
	// turn `kill -s SIGTERM <id>` into a per-container blocking call.
	assert!(must_wait_after_kill("SIGKILL"));
	assert!(must_wait_after_kill("KILL"));
	assert!(must_wait_after_kill("kill"));
	assert!(must_wait_after_kill("9"));
	assert!(must_wait_after_kill("0"));
	for s in [
		"SIGTERM", "TERM", "SIGHUP", "HUP", "SIGINT", "INT", "1", "15", "64",
	] {
		assert!(!must_wait_after_kill(s), "{s} must not trigger a wait");
	}
	// An empty or whitespace-only signal: the validation upstream rejects
	// those with an error before this runs, but the helper still has to
	// return false rather than panic.
	assert!(!must_wait_after_kill(""));
	assert!(!must_wait_after_kill("   "));
}
