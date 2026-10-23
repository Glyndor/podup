use crate::error::{ComposeError, Result};

/// Resolve a compose `stop_signal:` value to its numeric `syscall.Signal`.
///
/// libpod's `SpecGenerator.stop_signal` is an integer; sending the signal name
/// as a string returns HTTP 500. Accepts a bare number (`"15"`), a signal name
/// with or without the `SIG` prefix (`"SIGTERM"`, `"term"`), case-insensitively.
/// An unrecognized name returns a clear [`ComposeError::Unsupported`] rather than
/// silently dropping the value.
pub(in crate::engine::container) fn resolve_stop_signal(signal: &str) -> Result<i64> {
	let trimmed = signal.trim();
	if let Ok(num) = trimmed.parse::<i64>() {
		return Ok(num);
	}
	let upper = trimmed.to_ascii_uppercase();
	let name = upper.strip_prefix("SIG").unwrap_or(&upper);
	signal_number(name)
		.ok_or_else(|| ComposeError::Unsupported(format!("unknown stop_signal '{signal}'")))
}

/// First realtime signal number on Linux/glibc (`SIGRTMIN`).
const SIGRTMIN: i64 = 34;
/// Last realtime signal number on Linux/glibc (`SIGRTMAX`).
const SIGRTMAX: i64 = 64;

/// Map a bare (no `SIG` prefix) upper-case signal name to its Linux number.
///
/// In addition to the named POSIX signals, the realtime signals are resolved:
/// `RTMIN`/`RTMAX` and the offset forms `RTMIN+N`/`RTMAX-N`, matching how
/// docker-compose and Podman interpret them. The computed number must stay within
/// the realtime range [`SIGRTMIN`]..=[`SIGRTMAX`]; an out-of-range offset (e.g.
/// `RTMIN+40`) resolves to `None` so the caller reports it as unknown.
fn signal_number(name: &str) -> Option<i64> {
	if let Some(n) = realtime_signal_number(name) {
		return Some(n);
	}
	let n = match name {
		"HUP" => 1,
		"INT" => 2,
		"QUIT" => 3,
		"ILL" => 4,
		"TRAP" => 5,
		"ABRT" | "IOT" => 6,
		"BUS" => 7,
		"FPE" => 8,
		"KILL" => 9,
		"USR1" => 10,
		"SEGV" => 11,
		"USR2" => 12,
		"PIPE" => 13,
		"ALRM" => 14,
		"TERM" => 15,
		"STKFLT" => 16,
		"CHLD" | "CLD" => 17,
		"CONT" => 18,
		"STOP" => 19,
		"TSTP" => 20,
		"TTIN" => 21,
		"TTOU" => 22,
		"URG" => 23,
		"XCPU" => 24,
		"XFSZ" => 25,
		"VTALRM" => 26,
		"PROF" => 27,
		"WINCH" => 28,
		"IO" | "POLL" => 29,
		"PWR" => 30,
		"SYS" => 31,
		_ => return None,
	};
	Some(n)
}

/// Resolve a realtime signal name (`RTMIN`, `RTMAX`, `RTMIN+N`, `RTMAX-N`) to its
/// number, or `None` if `name` is not a realtime form or the computed number
/// falls outside the realtime range [`SIGRTMIN`]..=[`SIGRTMAX`].
fn realtime_signal_number(name: &str) -> Option<i64> {
	let (base, rest) = if let Some(rest) = name.strip_prefix("RTMIN") {
		(SIGRTMIN, rest)
	} else {
		let rest = name.strip_prefix("RTMAX")?;
		(SIGRTMAX, rest)
	};
	let number = if rest.is_empty() {
		base
	} else {
		// An offset must be `+N` (only valid after RTMIN) or `-N` (only valid
		// after RTMAX), matching POSIX/glibc's `SIGRTMIN+n` / `SIGRTMAX-n`.
		let (sign, digits) = rest.split_at(1);
		let offset: i64 = digits.parse().ok()?;
		match (base == SIGRTMIN, sign) {
			(true, "+") => base + offset,
			(false, "-") => base - offset,
			_ => return None,
		}
	};
	(SIGRTMIN..=SIGRTMAX).contains(&number).then_some(number)
}
