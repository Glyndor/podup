//! `--tail` / `--since` / `--until` value validation for `podup logs`.
//!
//! Pure so the predicates are unit-tested and the user sees a clear local
//! message instead of a raw podman HTTP 400.

use crate::error::{ComposeError, Result};

use super::options::LogsOptions;

/// Validate the `--tail`/`--since`/`--until` values client-side so a typo is
/// rejected with a clear local message instead of a raw podman HTTP 400. `tail`
/// must be `all` or a non-negative integer; `since`/`until` must be a Unix
/// timestamp or a Go-style duration (e.g. `10m`, `1h30m`) or an RFC3339-ish
/// timestamp. Pure so it is unit-tested.
pub(super) fn validate_log_filters(opts: &LogsOptions) -> Result<()> {
	if let Some(tail) = &opts.tail {
		if tail != "all" && tail.parse::<u64>().is_err() {
			return Err(ComposeError::Unsupported(format!(
				"invalid --tail value {tail:?}: expected a non-negative integer or 'all'"
			)));
		}
	}
	for (flag, value) in [("--since", &opts.since), ("--until", &opts.until)] {
		if let Some(v) = value {
			if !is_valid_log_time(v) {
				return Err(ComposeError::Unsupported(format!(
					"invalid {flag} value {v:?}: expected a duration (e.g. 10m, 1h30m), a Unix \
					 timestamp, or an RFC3339 time"
				)));
			}
		}
	}
	Ok(())
}

/// Whether a `--since`/`--until` value is a plausible duration, Unix timestamp,
/// or timestamp string. Conservative: rejects obvious garbage (`abc`) while
/// accepting the forms podman understands.
pub(super) fn is_valid_log_time(v: &str) -> bool {
	if v.is_empty() {
		return false;
	}
	// Unix timestamp (optionally fractional).
	if v.parse::<f64>().is_ok() {
		return true;
	}
	// Go-style duration: digit-run + unit, repeated (e.g. 1h30m, 90s, 500ms).
	if is_go_duration(v) {
		return true;
	}
	// Timestamp-ish: starts with a 4-digit year and contains only the characters
	// an RFC3339/date string uses. The server does the precise parse; this just
	// blocks free-form garbage.
	let bytes = v.as_bytes();
	bytes.len() >= 4
		&& bytes[..4].iter().all(u8::is_ascii_digit)
		&& v.chars().all(|c| {
			c.is_ascii_digit() || matches!(c, '-' | ':' | 't' | 'T' | 'z' | 'Z' | '.' | '+' | ' ')
		})
}

/// Match a Go-style duration: one or more `<number><unit>` segments, units one
/// of `ns,us,µs,ms,s,m,h`.
pub(super) fn is_go_duration(v: &str) -> bool {
	let mut rest = v.strip_prefix('-').unwrap_or(v);
	if rest.is_empty() {
		return false;
	}
	let mut segments = 0;
	while !rest.is_empty() {
		let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
		if digits.len() == rest.len() {
			// No digits consumed → not a duration segment.
			return false;
		}
		rest = digits;
		let unit_len = ["ms", "ns", "us", "µs", "s", "m", "h"]
			.into_iter()
			.find(|u| rest.starts_with(u))
			.map(str::len);
		match unit_len {
			Some(n) => rest = &rest[n..],
			None => return false,
		}
		segments += 1;
	}
	segments > 0
}

#[cfg(test)]
#[path = "logs_filters_tests.rs"]
mod tests;
