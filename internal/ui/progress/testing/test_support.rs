//! Fixtures the tests of several `progress` modules share.
//!
//! The session and the plain-sink buffer are process-global, so a test that
//! drives one and a test that drains the other collide unless they take turns.

/// The plain-sink tests share one process-wide session and buffer, and the
/// test harness runs them on separate threads: without this lock two of them
/// reset the buffer under each other and the count assertions fail on some
/// runs (3 of 20 measured in parallel, 0 of 43 single-threaded). A poisoned
/// lock is still a lock, so a failed test does not take the rest with it.
static SESSION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(in super::super) fn hold_session() -> std::sync::MutexGuard<'static, ()> {
	SESSION_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Drain any leftover entries in the plain-sink buffer so the next test starts
/// from a known state. The buffer is process-global; tests that exercise the
/// transitional/final interaction must call this before and after, otherwise
/// the order in which `cargo test` runs them leaks state.
pub(in super::super) fn reset_plain_buffer() {
	crate::ui::progress::buffer_drain_for_tests();
}
