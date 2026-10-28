//! Memory and CPU value parsers shared by the engine and tests.

/// Parse a memory string like `128m`, `1G`, `1024k`, `512b`, or a bare
/// number-of-bytes.
///
/// Recognised suffixes (case-insensitive): `b`, `k`/`kb`, `m`/`mb`,
/// `g`/`gb`, `t`/`tb`.  Returns `None` for unparseable values, and `Some(-1)`
/// for the special string `"-1"` (commonly used to disable swap limits).
pub fn parse_memory(s: &str) -> Option<i64> {
	let trimmed = s.trim();
	if trimmed.is_empty() {
		return None;
	}
	if trimmed == "-1" {
		return Some(-1);
	}

	// Find where the numeric portion ends.
	let split_at = trimmed
		.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
		.unwrap_or(trimmed.len());

	let (num_part, suffix) = trimmed.split_at(split_at);
	let num_part = num_part.trim();
	let suffix = suffix.trim().to_ascii_lowercase();

	let num: f64 = num_part.parse().ok()?;
	if num < 0.0 {
		return None;
	}

	let multiplier: u64 = match suffix.as_str() {
		"" | "b" => 1,
		"k" | "kb" => 1024,
		"m" | "mb" => 1024 * 1024,
		"g" | "gb" => 1024 * 1024 * 1024,
		"t" | "tb" => 1024_u64 * 1024 * 1024 * 1024,
		_ => return None,
	};

	let bytes = num * multiplier as f64;
	// `i64::MAX as f64` rounds up to 2^63, which is not an `i64`: `>=`, so the
	// cast below never saturates to `i64::MAX` and hands back a limit the
	// input did not ask for.
	if bytes >= i64::MAX as f64 {
		return None;
	}
	Some(bytes as i64)
}

/// Parse a CPU count string like `"0.5"`, `"1"`, `"2.5"` into nano-CPUs
/// (1 CPU = 1_000_000_000 nano-CPUs).
///
/// Returns `None` for non-finite (`NaN`/`inf`), negative, or out-of-`i64`-range
/// values instead of silently saturating or wrapping the cast.
pub fn parse_cpus(s: &str) -> Option<i64> {
	let nanos = s.trim().parse::<f64>().ok()? * 1e9;
	// `>=` for the same reason as in `parse_memory`.
	if !nanos.is_finite() || nanos < 0.0 || nanos >= i64::MAX as f64 {
		return None;
	}
	Some(nanos as i64)
}

/// Parse a duration like `5s`, `200ms`, `1m`, `1h` into seconds (rounded down).
///
/// This is a best-effort parser used when the engine needs an integer
/// seconds value (e.g. Docker API `start_period`).
pub fn parse_duration_secs(s: &str) -> Option<u64> {
	let nanos = duration_nanos(s)?;
	let secs = nanos / NANOS_PER_SEC;
	u64::try_from(secs).ok()
}

/// Parse a duration into seconds, rounding the fraction up.
///
/// A timeout written in a fraction of a second must not become zero:
/// `500ms` is half a second of grace, not an instant kill. Rounding up keeps
/// the timeout at least as long as what was asked for.
pub fn parse_duration_secs_rounded_up(s: &str) -> Option<u64> {
	let nanos = duration_nanos(s)?;
	let secs = nanos.div_ceil(NANOS_PER_SEC);
	u64::try_from(secs).ok()
}

/// Parse a duration like `5s`, `200ms`, `1m`, `1h` into nanoseconds (used by
/// Docker healthcheck APIs).
///
/// Returns `None` for non-finite (`NaN`/`inf`), negative, or out-of-`i64`-range
/// results instead of saturating or wrapping the cast.
pub fn parse_duration_nanos(s: &str) -> Option<i64> {
	let nanos = duration_nanos(s)?;
	i64::try_from(nanos).ok()
}

/// Nanoseconds in a second.
const NANOS_PER_SEC: u128 = 1_000_000_000;

/// The number of nanoseconds each duration unit is worth.
///
/// The set is the one the compose specification uses: nanoseconds,
/// microseconds, milliseconds, seconds, minutes and hours. A number without a
/// unit is a number of seconds.
fn duration_unit_nanos(unit: &str) -> Option<u128> {
	Some(match unit {
		"ns" => 1,
		"us" | "µs" => 1_000,
		"ms" => 1_000_000,
		"s" => NANOS_PER_SEC,
		"m" => 60 * NANOS_PER_SEC,
		"h" => 3_600 * NANOS_PER_SEC,
		_ => return None,
	})
}

/// Parse a duration into nanoseconds.
///
/// The syntax is a sequence of number-and-unit pairs, as in Go's
/// `time.ParseDuration` and as the compose specification writes them
/// (`stop_grace_period: 1m30s`). A number without a unit is a number of
/// seconds, wherever it appears.
///
/// The arithmetic is exact and in integers: the total is counted in
/// nanoseconds and never passes through a float, so a value near the limit of
/// the range is not rounded into it. The total is rounded up to the next
/// nanosecond, so a duration asked for is never shortened.
fn duration_nanos(text: &str) -> Option<u128> {
	let text = text.trim();
	if text.is_empty() {
		return None;
	}
	let bytes = text.as_bytes();
	let mut at = 0usize;
	let mut total: u128 = 0;
	let mut parts = 0usize;
	while at < bytes.len() {
		// The number: digits and at most one decimal point.
		let number_start = at;
		while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'.') {
			at += 1;
		}
		if at == number_start {
			return None;
		}
		let number = &text[number_start..at];
		// The unit: everything up to the next number.
		let unit_start = at;
		while at < bytes.len() && !bytes[at].is_ascii_digit() && bytes[at] != b'.' {
			at += 1;
		}
		let unit = text[unit_start..at].trim();
		let scale = if unit.is_empty() {
			NANOS_PER_SEC
		} else {
			duration_unit_nanos(unit)?
		};
		total = total.checked_add(number_nanos(number, scale)?)?;
		parts += 1;
	}
	if parts == 0 {
		return None;
	}
	Some(total)
}

/// The nanoseconds a decimal number is worth at the given scale.
///
/// The fraction is counted exactly and rounded up, so `0.5s` is 500 ms and
/// `1.5ms` is 1.5 ms (1 500 000 ns) rather than a float approximation of it.
fn number_nanos(number: &str, scale: u128) -> Option<u128> {
	let (whole, fraction) = match number.split_once('.') {
		Some((whole, fraction)) => (whole, fraction),
		None => (number, ""),
	};
	if whole.is_empty() && fraction.is_empty() {
		return None;
	}
	if !whole.bytes().all(|b| b.is_ascii_digit()) || !fraction.bytes().all(|b| b.is_ascii_digit()) {
		return None;
	}
	let whole: u128 = if whole.is_empty() {
		0
	} else {
		whole.parse().ok()?
	};
	let nanos = whole.checked_mul(scale)?;
	if fraction.is_empty() {
		return Some(nanos);
	}
	// A fraction finer than 10^-18 of the unit cannot be held in the counter,
	// but it is still a fraction of a nanosecond: it rounds up to one.
	if fraction.len() > 18 {
		let finer = fraction[18..].bytes().any(|b| b != b'0');
		return Some(nanos + u128::from(finer));
	}
	let digits: u128 = fraction.parse().ok()?;
	let divisor = 10u128.checked_pow(fraction.len() as u32)?;
	let numerator = digits.checked_mul(scale)?;
	let mut fraction_nanos = numerator / divisor;
	if numerator % divisor != 0 {
		fraction_nanos += 1;
	}
	nanos.checked_add(fraction_nanos)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "size_tests.rs"]
mod tests;
