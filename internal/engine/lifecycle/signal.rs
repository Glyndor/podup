//! Validation for `kill -s/--signal` values.
//!
//! `kill` forwards the requested signal to libpod as a `signal=` query
//! parameter. An empty (or whitespace-only) value renders as `signal=`, which
//! libpod silently treats as the default SIGKILL — so an unset shell variable
//! passed via `-s "$SIG"` would destroy every targeted container with no
//! warning. To match `docker compose`'s up-front validation, the signal is
//! checked here before any request is issued.

use crate::error::{ComposeError, Result};

/// Signal names podup accepts for `kill -s`, matched case-insensitively and
/// with the `SIG` prefix optional. Covers the standard POSIX signals plus the
/// common Linux additions Podman understands.
const KNOWN_SIGNALS: &[&str] = &[
	"ABRT", "ALRM", "BUS", "CHLD", "CLD", "CONT", "EMT", "FPE", "HUP", "ILL", "INT", "IO", "IOT",
	"KILL", "LOST", "PIPE", "POLL", "PROF", "PWR", "QUIT", "RTMAX", "RTMIN", "SEGV", "STKFLT",
	"STOP", "SYS", "TERM", "TRAP", "TSTP", "TTIN", "TTOU", "UNUSED", "URG", "USR1", "USR2",
	"VTALRM", "WINCH", "XCPU", "XFSZ",
];

/// Validate a `kill` signal before it is forwarded to libpod.
///
/// Accepts a numeric signal in `1..=64` or a known signal name (case-insensitive,
/// with or without the `SIG` prefix). Rejects an empty/whitespace-only value and
/// any unrecognised name/number with [`ComposeError::InvalidSignal`], rather than
/// letting it default to SIGKILL on the libpod side.
pub(crate) fn validate_signal(signal: &str) -> Result<()> {
	let trimmed = signal.trim();
	if trimmed.is_empty() {
		return Err(ComposeError::InvalidSignal(
			"signal must not be empty".into(),
		));
	}
	// A bare number is a raw signal number; restrict it to the valid range.
	if trimmed.chars().all(|c| c.is_ascii_digit()) {
		return match trimmed.parse::<u32>() {
			Ok(n) if (1..=64).contains(&n) => Ok(()),
			_ => Err(ComposeError::InvalidSignal(signal.into())),
		};
	}
	let upper = trimmed.to_ascii_uppercase();
	let name = upper.strip_prefix("SIG").unwrap_or(&upper);
	if KNOWN_SIGNALS.contains(&name) {
		Ok(())
	} else {
		Err(ComposeError::InvalidSignal(signal.into()))
	}
}

#[cfg(test)]
mod tests {
	use super::validate_signal;
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
}
