//! Tests for the memory / cpu / duration parser helpers.

use podup::size::{parse_cpus, parse_duration_nanos, parse_duration_secs, parse_memory};

#[test]
fn memory_bytes() {
    assert_eq!(parse_memory("1024"), Some(1024));
    assert_eq!(parse_memory("1024b"), Some(1024));
    assert_eq!(parse_memory("1024B"), Some(1024));
}

#[test]
fn memory_kilobytes() {
    assert_eq!(parse_memory("1k"), Some(1024));
    assert_eq!(parse_memory("1K"), Some(1024));
    assert_eq!(parse_memory("2KB"), Some(2 * 1024));
}

#[test]
fn memory_megabytes() {
    assert_eq!(parse_memory("128m"), Some(128 * 1024 * 1024));
    assert_eq!(parse_memory("128M"), Some(128 * 1024 * 1024));
    assert_eq!(parse_memory("128MB"), Some(128 * 1024 * 1024));
}

#[test]
fn memory_gigabytes() {
    assert_eq!(parse_memory("1g"), Some(1024 * 1024 * 1024));
    assert_eq!(parse_memory("1G"), Some(1024 * 1024 * 1024));
}

#[test]
fn memory_terabytes() {
    assert_eq!(parse_memory("1t"), Some(1024_i64 * 1024 * 1024 * 1024));
}

#[test]
fn memory_negative_one() {
    assert_eq!(parse_memory("-1"), Some(-1));
}

#[test]
fn memory_invalid() {
    assert!(parse_memory("xyz").is_none());
    assert!(parse_memory("12xb").is_none());
}

#[test]
fn memory_overflow_returns_none() {
    // Values that overflow i64::MAX must return None, not wrap around.
    assert!(parse_memory("99999999t").is_none());
    assert!(parse_memory("9999999999g").is_none());
}

#[test]
fn cpus_fractional() {
    assert_eq!(parse_cpus("0.5"), Some(500_000_000));
    assert_eq!(parse_cpus("1"), Some(1_000_000_000));
    assert_eq!(parse_cpus("2.5"), Some(2_500_000_000));
}

#[test]
fn duration_seconds() {
    assert_eq!(parse_duration_secs("5"), Some(5));
    assert_eq!(parse_duration_secs("5s"), Some(5));
    assert_eq!(parse_duration_secs("1m"), Some(60));
    assert_eq!(parse_duration_secs("1h"), Some(3600));
}

#[test]
fn duration_nanos() {
    assert_eq!(parse_duration_nanos("1s"), Some(1_000_000_000));
    assert_eq!(parse_duration_nanos("1ms"), Some(1_000_000));
    assert_eq!(parse_duration_nanos("500ms"), Some(500_000_000));
}
