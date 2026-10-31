use super::InitCleanup;
use std::cell::Cell;

#[test]
fn pre_start_cancellation_invokes_cleanup() {
	let removed = Cell::new(0);
	let guard = InitCleanup {
		remove: Some(|| removed.set(removed.get() + 1)),
	};
	drop(guard);
	assert_eq!(removed.get(), 1);
}

#[test]
fn pre_start_completed_removal_disarms_cleanup() {
	let removed = Cell::new(false);
	let mut guard = InitCleanup {
		remove: Some(|| removed.set(true)),
	};
	guard.disarm();
	drop(guard);
	assert!(!removed.get());
}
