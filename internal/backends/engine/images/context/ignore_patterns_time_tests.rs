//! Patterns made of stacked wildcards must finish in polynomial time.
//!
//! Each case runs on its own thread with a generous budget, so a regression
//! to the exponential matcher fails the test instead of hanging the suite.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::ignore_patterns::{glob_match, is_ignored};

const BUDGET: Duration = Duration::from_secs(2);

fn within(f: impl FnOnce() -> bool + Send + 'static) -> Option<(bool, Duration)> {
	let (tx, rx) = mpsc::channel();
	let start = Instant::now();
	std::thread::spawn(move || {
		let _ = tx.send(f());
	});
	rx.recv_timeout(BUDGET).ok().map(|r| (r, start.elapsed()))
}

#[test]
fn a_stack_of_stars_finishes_quickly() {
	let pattern = format!("{}b", "*a".repeat(12));
	let name = "a".repeat(60);
	let (matched, _) = within(move || glob_match(&pattern, &name)).expect("glob_match timed out");
	assert!(!matched);
}

#[test]
fn a_stack_of_double_stars_finishes_quickly() {
	let pattern = format!("{}b", "**/".repeat(10));
	let path = "a/".repeat(40) + "a";
	let patterns = vec![pattern];
	let (matched, _) = within(move || is_ignored(&path, &patterns)).expect("is_ignored timed out");
	assert!(!matched);
}

#[test]
fn a_much_longer_stack_still_finishes_quickly() {
	let pattern = format!("{}b", "*a".repeat(500));
	let name = "a".repeat(2000);
	let (matched, _) = within(move || glob_match(&pattern, &name)).expect("glob_match timed out");
	assert!(!matched);
	let pattern = format!("{}b", "**/*a".repeat(300));
	let path = "a/".repeat(500) + "a";
	let (matched, _) = within(move || glob_match(&pattern, &path)).expect("glob_match timed out");
	assert!(!matched);
}

#[test]
fn a_stack_that_does_match_still_matches() {
	let pattern = format!("{}b", "*a".repeat(12));
	let name = format!("{}b", "a".repeat(60));
	let (matched, _) = within(move || glob_match(&pattern, &name)).expect("glob_match timed out");
	assert!(matched);
	let pattern = format!("{}b", "**/".repeat(10));
	let path = "a/".repeat(40) + "b";
	let (matched, _) = within(move || glob_match(&pattern, &path)).expect("glob_match timed out");
	assert!(matched);
}
