//! Validation for `kill -s/--signal` values.
//!
//! `kill` forwards the requested signal to libpod as a `signal=` query
//! parameter. An empty (or whitespace-only) value renders as `signal=`, which
//! libpod silently treats as the default SIGKILL, so an unset shell variable
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

/// Whether a `kill` whose target signal is `signal` must be followed by a
/// `wait?condition=stopped` to match the Docker compat handler's behaviour.
///
/// The compat handler blocks until the container has exited or stopped on
/// signals the kernel delivers unconditionally: SIGKILL (`9`, `KILL`,
/// `SIGKILL`) and signal `0` (the existence probe). libpod's
/// `/containers/{id}/kill` answers immediately for every signal, so a caller
/// that relied on the compat handler's wait would observe a still-running
/// container when `kill` returned. SIGTERM and the other "graceful" signals
/// stay fast on the libpod side; adding a wait there would pin every `kill
/// -s SIGTERM` behind every targeted container even when nothing required
/// it. Pure so the rule is unit-tested without a socket.
pub(crate) fn must_wait_after_kill(signal: &str) -> bool {
	let trimmed = signal.trim();
	if trimmed.is_empty() {
		return false;
	}
	if trimmed.chars().all(|c| c.is_ascii_digit()) {
		return matches!(trimmed.parse::<u32>(), Ok(0) | Ok(9));
	}
	let upper = trimmed.to_ascii_uppercase();
	let name = upper.strip_prefix("SIG").unwrap_or(&upper);
	name == "KILL"
}

#[cfg(test)]
#[path = "signal_tests.rs"]
mod tests;
