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
	if bytes > i64::MAX as f64 {
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
	if !nanos.is_finite() || nanos < 0.0 || nanos > i64::MAX as f64 {
		return None;
	}
	Some(nanos as i64)
}

/// Parse a duration like `5s`, `200ms`, `1m`, `1h` into seconds (rounded down).
///
/// This is a best-effort parser used when the engine needs an integer
/// seconds value (e.g. Docker API `start_period`).
pub fn parse_duration_secs(s: &str) -> Option<u64> {
	let trimmed = s.trim();
	if trimmed.is_empty() {
		return None;
	}
	let split_at = trimmed
		.find(|c: char| !(c.is_ascii_digit() || c == '.'))
		.unwrap_or(trimmed.len());
	let (num_part, suffix) = trimmed.split_at(split_at);
	let num: f64 = num_part.parse().ok()?;
	let secs = match suffix.trim() {
		"" | "s" => num,
		"ms" => num / 1000.0,
		"us" | "µs" => num / 1_000_000.0,
		"ns" => num / 1_000_000_000.0,
		"m" => num * 60.0,
		"h" => num * 3600.0,
		_ => return None,
	};
	Some(secs as u64)
}

/// Parse a duration like `5s`, `200ms`, `1m`, `1h` into nanoseconds (used by
/// Docker healthcheck APIs).
///
/// Returns `None` for non-finite (`NaN`/`inf`), negative, or out-of-`i64`-range
/// results instead of saturating or wrapping the cast.
pub fn parse_duration_nanos(s: &str) -> Option<i64> {
	let trimmed = s.trim();
	if trimmed.is_empty() {
		return None;
	}
	let split_at = trimmed
		.find(|c: char| !(c.is_ascii_digit() || c == '.'))
		.unwrap_or(trimmed.len());
	let (num_part, suffix) = trimmed.split_at(split_at);
	let num: f64 = num_part.parse().ok()?;
	let nanos = match suffix.trim() {
		"" | "s" => num * 1e9,
		"ms" => num * 1e6,
		"us" | "µs" => num * 1e3,
		"ns" => num,
		"m" => num * 60.0 * 1e9,
		"h" => num * 3600.0 * 1e9,
		_ => return None,
	};
	// Reject non-finite or out-of-range results rather than saturating the cast.
	if !nanos.is_finite() || nanos < 0.0 || nanos > i64::MAX as f64 {
		return None;
	}
	Some(nanos as i64)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	// parse_memory

	#[test]
	fn memory_bare_bytes() {
		assert_eq!(parse_memory("1024"), Some(1024));
	}

	#[test]
	fn memory_suffix_b() {
		assert_eq!(parse_memory("512b"), Some(512));
	}

	#[test]
	fn memory_suffix_k() {
		assert_eq!(parse_memory("4k"), Some(4 * 1024));
	}

	#[test]
	fn memory_suffix_kb() {
		assert_eq!(parse_memory("4kb"), Some(4 * 1024));
	}

	#[test]
	fn memory_suffix_m() {
		assert_eq!(parse_memory("128m"), Some(128 * 1024 * 1024));
	}

	#[test]
	fn memory_suffix_mb() {
		assert_eq!(parse_memory("128mb"), Some(128 * 1024 * 1024));
	}

	#[test]
	fn memory_suffix_g() {
		assert_eq!(parse_memory("1g"), Some(1024 * 1024 * 1024));
	}

	#[test]
	fn memory_suffix_uppercase() {
		assert_eq!(parse_memory("64M"), Some(64 * 1024 * 1024));
	}

	#[test]
	fn memory_minus_one_special() {
		assert_eq!(parse_memory("-1"), Some(-1));
	}

	#[test]
	fn memory_empty_is_none() {
		assert_eq!(parse_memory(""), None);
	}

	#[test]
	fn memory_invalid_is_none() {
		assert_eq!(parse_memory("abc"), None);
	}

	#[test]
	fn memory_unknown_suffix_is_none() {
		assert_eq!(parse_memory("100x"), None);
	}

	#[test]
	fn memory_negative_is_none() {
		// A negative memory size is rejected rather than wrapping.
		assert_eq!(parse_memory("-1g"), None);
	}

	// parse_cpus

	#[test]
	fn cpus_integer() {
		assert_eq!(parse_cpus("2"), Some(2_000_000_000));
	}

	#[test]
	fn cpus_fraction() {
		assert_eq!(parse_cpus("0.5"), Some(500_000_000));
	}

	#[test]
	fn cpus_empty_is_none() {
		assert_eq!(parse_cpus(""), None);
	}

	#[test]
	fn cpus_non_finite_is_none() {
		assert_eq!(parse_cpus("nan"), None);
		assert_eq!(parse_cpus("inf"), None);
		assert_eq!(parse_cpus("1e300"), None);
	}

	#[test]
	fn cpus_negative_is_none() {
		assert_eq!(parse_cpus("-1"), None);
	}

	// parse_duration_secs

	#[test]
	fn duration_secs_plain_s() {
		assert_eq!(parse_duration_secs("30s"), Some(30));
	}

	#[test]
	fn duration_secs_minutes() {
		assert_eq!(parse_duration_secs("2m"), Some(120));
	}

	#[test]
	fn duration_secs_hours() {
		assert_eq!(parse_duration_secs("1h"), Some(3600));
	}

	#[test]
	fn duration_secs_milliseconds_truncates() {
		assert_eq!(parse_duration_secs("500ms"), Some(0));
	}

	#[test]
	fn duration_secs_bare_number() {
		assert_eq!(parse_duration_secs("10"), Some(10));
	}

	#[test]
	fn duration_secs_empty_is_none() {
		assert_eq!(parse_duration_secs(""), None);
	}

	#[test]
	fn duration_secs_unknown_suffix_is_none() {
		assert_eq!(parse_duration_secs("5d"), None);
	}

	// parse_duration_nanos

	#[test]
	fn duration_nanos_seconds() {
		assert_eq!(parse_duration_nanos("1s"), Some(1_000_000_000));
	}

	#[test]
	fn duration_nanos_milliseconds() {
		assert_eq!(parse_duration_nanos("200ms"), Some(200_000_000));
	}

	#[test]
	fn duration_nanos_minutes() {
		assert_eq!(parse_duration_nanos("1m"), Some(60 * 1_000_000_000));
	}

	#[test]
	fn duration_nanos_nanoseconds() {
		assert_eq!(parse_duration_nanos("500ns"), Some(500));
	}

	#[test]
	fn duration_nanos_overflow_is_none() {
		// A large finite value whose nanosecond product exceeds i64::MAX must
		// return None rather than saturating to i64::MAX.
		assert_eq!(parse_duration_nanos("99999999999h"), None);
	}

	#[test]
	fn duration_nanos_micros_and_hours() {
		assert_eq!(parse_duration_nanos("3us"), Some(3_000));
		assert_eq!(parse_duration_nanos("2h"), Some(2 * 3600 * 1_000_000_000));
	}

	#[test]
	fn duration_nanos_unknown_suffix_is_none() {
		assert_eq!(parse_duration_nanos("5days"), None);
	}
}
