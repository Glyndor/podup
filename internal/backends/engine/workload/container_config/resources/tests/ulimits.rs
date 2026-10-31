use super::*;

// --- ulimits ---

#[test]
fn build_ulimits_single_value() {
	use crate::compose::types::UlimitConfig;
	let mut svc = default_service();
	svc.ulimits
		.insert("nofile".to_string(), UlimitConfig::Single(1024));
	let ul = build_ulimits(&svc);
	assert_eq!(ul.len(), 1);
	assert_eq!(ul[0].ulimit_type, "nofile");
	assert_eq!(ul[0].soft, 1024);
	assert_eq!(ul[0].hard, 1024);
}

#[test]
fn build_ulimits_pair() {
	use crate::compose::types::UlimitConfig;
	let mut svc = default_service();
	svc.ulimits.insert(
		"nofile".to_string(),
		UlimitConfig::Pair {
			soft: 512,
			hard: 2048,
		},
	);
	let ul = build_ulimits(&svc);
	assert_eq!(ul[0].soft, 512);
	assert_eq!(ul[0].hard, 2048);
}

#[test]
fn build_ulimits_clamps_soft_above_hard() {
	use crate::compose::types::UlimitConfig;
	let mut svc = default_service();
	svc.ulimits.insert(
		"nofile".to_string(),
		UlimitConfig::Pair {
			soft: 65535,
			hard: 1024,
		},
	);
	let ul = build_ulimits(&svc);
	assert_eq!(ul[0].soft, 1024, "soft must be clamped down to hard");
	assert_eq!(ul[0].hard, 1024);
}

#[test]
fn build_ulimits_rejects_unknown_resource_name() {
	use crate::compose::types::UlimitConfig;
	let mut svc = default_service();
	svc.ulimits
		.insert("bogus,inject=1".to_string(), UlimitConfig::Single(1024));
	assert!(
		build_ulimits(&svc).is_empty(),
		"an unknown ulimit name must be dropped, not forwarded"
	);
}

#[test]
fn ulimit_pair_treats_unlimited_as_the_largest_and_never_wraps() {
	assert_eq!(ulimit_pair(512, -1, "nofile"), (512, u64::MAX));
	assert_eq!(ulimit_pair(-1, 2048, "nofile"), (2048, 2048));
	assert_eq!(ulimit_pair(-1, -1, "nofile"), (u64::MAX, u64::MAX));
	assert_eq!(ulimit_pair(i64::MIN, 7, "nofile"), (0, 7));
	assert_eq!(ulimit_pair(i64::MAX, i64::MIN, "nofile"), (0, 0));
}

// --- ulimit value conversion ---

#[test]
fn ulimit_minus_one_is_unlimited() {
	assert_eq!(ulimit_value(-1, "nofile", "soft"), u64::MAX);
}

#[test]
fn ulimit_other_negative_clamped_to_zero() {
	// Must not wrap to a huge u64 via `as`.
	assert_eq!(ulimit_value(-5, "nofile", "soft"), 0);
}

#[test]
fn ulimit_positive_passes_through() {
	assert_eq!(ulimit_value(1024, "nofile", "hard"), 1024);
}
