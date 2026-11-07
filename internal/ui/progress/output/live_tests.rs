use super::*;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Two regions open together must not stack handlers, since each would race to
/// call `process::exit`; and once the region that owns the handler is gone, so
/// is the handler: a Ctrl-C during what the command does next is not its to
/// answer (`up` attaches after its board closes, and attach stops the project
/// on that very signal).
#[tokio::test]
async fn the_interrupt_handler_lives_exactly_as_long_as_its_region() {
	static LATCH: AtomicBool = AtomicBool::new(false);
	let interrupt = Arc::new(tokio::sync::Notify::new());
	let fired = Arc::new(AtomicBool::new(false));
	let handler = |wait: Arc<tokio::sync::Notify>, fired: Arc<AtomicBool>| {
		install(&LATCH, async move { wait.notified().await }, move || {
			fired.store(true, Ordering::SeqCst)
		})
	};
	let first = handler(interrupt.clone(), fired.clone()).expect("the first installs");
	assert!(
		handler(interrupt.clone(), fired.clone()).is_none(),
		"the second must not"
	);
	drop(first);
	assert!(!LATCH.load(Ordering::Relaxed), "dropping frees the latch");
	interrupt.notify_one();
	tokio::task::yield_now().await;
	tokio::time::sleep(std::time::Duration::from_millis(20)).await;
	assert!(
		!fired.load(Ordering::SeqCst),
		"a handler whose region is gone answered an interrupt"
	);
	// And while it lives, it answers.
	let live = handler(interrupt.clone(), fired.clone()).expect("free again");
	interrupt.notify_one();
	tokio::time::sleep(std::time::Duration::from_millis(50)).await;
	assert!(fired.load(Ordering::SeqCst), "a live handler must answer");
	drop(live);
}

/// The four sequences are what the repaint arithmetic is built on, so they
/// are pinned rather than left to a typo. `\x1b[3A` moves up three; `\x1b[J`
/// erases from the cursor down.
#[test]
fn the_cursor_moves_are_what_they_claim() {
	assert_eq!(cursor_up(3), "\u{1b}[3A");
	assert_eq!(CLEAR_BELOW, "\u{1b}[J");
	assert_eq!(HIDE_CURSOR, "\u{1b}[?25l");
	assert_eq!(SHOW_CURSOR, "\u{1b}[?25h");
}

/// The wiring, not only the mechanism: a `Region` installs the handler when it
/// starts and gives it back when it is dropped. `up` closes its board and
/// attaches; a handler that stayed behind would answer the interrupt that
/// attach is waiting for. The only test that builds a `Region`, because the
/// latch it reads is process-wide.
#[tokio::test]
async fn a_region_installs_the_handler_and_takes_it_away_when_it_is_dropped() {
	use std::sync::atomic::Ordering;
	assert!(!INSTALLED.load(Ordering::Relaxed), "nothing installed yet");
	let region = Region::new(Target::Stderr);
	assert!(
		INSTALLED.load(Ordering::Relaxed),
		"a region must hold the handler while it lives"
	);
	drop(region);
	assert!(
		!INSTALLED.load(Ordering::Relaxed),
		"the handler outlived the region that owned it"
	);
}
