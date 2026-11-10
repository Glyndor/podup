//! The budget refuses the entry that passes either limit, and says which.

use super::LinkBudget;

#[test]
fn the_entry_that_passes_the_entry_limit_is_refused() {
	let mut budget = LinkBudget::new(2, u64::MAX);
	budget.spend(0).expect("first");
	budget.spend(0).expect("second");
	let err = budget.spend(0).expect_err("third").to_string();
	assert!(err.contains("too many entries"), "{err}");
	assert!(err.contains("Copy without -L"), "{err}");
}

#[test]
fn the_entry_that_passes_the_byte_limit_is_refused() {
	let mut budget = LinkBudget::new(10, 5);
	budget.spend(5).expect("fits exactly");
	let err = budget.spend(1).expect_err("over").to_string();
	assert!(err.contains("bytes of content"), "{err}");
}

#[test]
fn the_default_budget_allows_a_normal_tree() {
	let mut budget = LinkBudget::default();
	for _ in 0..10_000 {
		budget.spend(4096).expect("a normal tree fits");
	}
}
