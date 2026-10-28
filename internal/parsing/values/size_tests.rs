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

// A duration written as more than one number and unit, the way the compose
// specification writes them (`stop_grace_period: 1m30s`).

#[test]
fn duration_secs_compound_minutes_and_seconds() {
	assert_eq!(parse_duration_secs("1m30s"), Some(90));
}

#[test]
fn duration_secs_compound_hours_and_minutes() {
	assert_eq!(parse_duration_secs("1h30m"), Some(5400));
}

#[test]
fn duration_nanos_compound_minutes_and_seconds() {
	assert_eq!(parse_duration_nanos("1m30s"), Some(90 * 1_000_000_000));
}

#[test]
fn duration_nanos_compound_hours_minutes_and_seconds() {
	assert_eq!(
		parse_duration_nanos("1h30m15s"),
		Some(5_415 * 1_000_000_000)
	);
}

#[test]
fn duration_compound_with_a_fraction() {
	assert_eq!(parse_duration_nanos("1.5m30s"), Some(120 * 1_000_000_000));
}

#[test]
fn duration_compound_rejects_an_unknown_unit() {
	assert_eq!(parse_duration_secs("1m30d"), None);
}

#[test]
fn duration_compound_ends_with_a_number_without_a_unit() {
	// A number without a unit is a number of seconds wherever it appears, the
	// way a lone one has always been read.
	assert_eq!(parse_duration_secs("1m30s5"), Some(95));
}

// The rounding-up variant, for the timeouts that must not become shorter
// than what was asked for.

#[test]
fn duration_secs_rounded_up_keeps_half_a_second() {
	assert_eq!(parse_duration_secs_rounded_up("500ms"), Some(1));
}

#[test]
fn duration_secs_rounded_up_leaves_a_whole_second_alone() {
	assert_eq!(parse_duration_secs_rounded_up("30s"), Some(30));
}

#[test]
fn duration_secs_rounded_up_rounds_a_fraction_up() {
	assert_eq!(parse_duration_secs_rounded_up("1.5s"), Some(2));
}

#[test]
fn duration_secs_rounded_up_reads_a_compound_duration() {
	assert_eq!(parse_duration_secs_rounded_up("1m30s"), Some(90));
}

#[test]
fn duration_secs_rounded_up_rejects_what_it_cannot_read() {
	assert_eq!(parse_duration_secs_rounded_up("5d"), None);
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

#[test]
fn duration_secs_out_of_range_is_none() {
	// A value whose seconds product exceeds u64::MAX must return None rather
	// than saturating the cast to u64::MAX.
	assert_eq!(parse_duration_secs("1e24"), None);
	assert_eq!(parse_duration_secs("1e24h"), None);
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

// --- the first out-of-range value is rejected, not saturated -------------
//
// `i64::MAX as f64` and `u64::MAX as f64` both round up to a power of two that
// the integer type cannot hold, so a `>` bound let exactly that value through
// and the `as` cast saturated it: the caller got `i64::MAX` or `u64::MAX` for
// an input that said something else. Found by the `duration` fuzz target.

#[test]
fn memory_of_exactly_two_to_the_63_is_none() {
	// 8589934592 * 2^30 = 2^63.
	assert_eq!(parse_memory("8589934592g"), None);
	assert_eq!(parse_memory("9223372036854775808"), None);
}

#[test]
fn memory_just_under_the_limit_still_parses() {
	assert_eq!(parse_memory("8589934591g"), Some(8_589_934_591_i64 << 30));
}

#[test]
fn duration_nanos_of_exactly_two_to_the_63_is_none() {
	assert_eq!(parse_duration_nanos("9223372036854775808ns"), None);
	assert_eq!(
		parse_duration_nanos("9223372036854774784ns"),
		Some(9_223_372_036_854_774_784)
	);
}

#[test]
fn duration_secs_of_exactly_two_to_the_64_is_none() {
	assert_eq!(parse_duration_secs("18446744073709551616s"), None);
	assert_eq!(
		parse_duration_secs("18446744073709549568s"),
		Some(18_446_744_073_709_549_568)
	);
}
