use super::*;

#[test]
fn replica_limit_default_and_env_override() {
	// One test owns the shared `PODUP_MAX_REPLICAS` env var for its whole body
	// so a sibling test running in parallel can never race it. The lock is
	// shared with `up/up_one_tests.rs`; taking it here makes the "one test owns"
	// rule real instead of aspirational.
	let _guard = MAX_REPLICAS_TEST_LOCK.lock().unwrap();
	let max = DEFAULT_MAX_REPLICAS as usize;

	// Default ceiling: at-limit allowed, over-limit rejected.
	std::env::remove_var("PODUP_MAX_REPLICAS");
	assert!(check_replica_limit("web", 1).is_ok());
	assert!(check_replica_limit("web", max).is_ok());
	let err = check_replica_limit("web", max + 1).unwrap_err();
	assert!(matches!(
		err,
		crate::error::ComposeError::ReplicaLimitExceeded { .. }
	));
	assert!(check_replica_limit("web", 100_000).is_err());

	// Env override lowers the ceiling.
	std::env::set_var("PODUP_MAX_REPLICAS", "2");
	assert!(check_replica_limit("web", 2).is_ok());
	assert!(check_replica_limit("web", 3).is_err());

	// A zero/garbage override falls back to the default ceiling.
	std::env::set_var("PODUP_MAX_REPLICAS", "0");
	assert!(check_replica_limit("web", max).is_ok());
	std::env::set_var("PODUP_MAX_REPLICAS", "nope");
	assert!(check_replica_limit("web", max).is_ok());
	std::env::remove_var("PODUP_MAX_REPLICAS");
}
