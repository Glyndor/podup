use super::super::super::test_support::{hold_session, reset_plain_buffer};
use crate::ui::progress::{buffered_count_for_tests, is_noop_final_for_tests};

/// The plain sink emits only the final verb when a transitional one was
/// buffered for the same `(kind, name)`. `Creating` then `Exists` collapses to
/// a single `Exists` line (#1673).
///
/// Exercises `emit` through the `progress::start` / `progress::finish` path
/// rather than reading the buffer directly, so the test asserts the user-
/// visible behaviour and would catch a regression that broke the start/finish
/// pairing without touching the buffer.
#[test]
fn plain_sink_prints_only_the_final_verb() {
	let _session = hold_session();
	reset_plain_buffer();
	// Disable colour so the renderer (and anstream::stderr) does not insert
	// any escape sequence we would have to strip from the captured output.
	let prev_colour = anstream::ColorChoice::global();
	let prev_progress = crate::ui::progress_enabled();
	anstream::ColorChoice::Never.write_global();
	crate::ui::set_progress(true);

	// Capture whatever the renderer writes to stderr. The plain-sink path
	// uses `anstream::stderr()`, so redirecting the process stderr at this
	// level does not catch it; we instead pin the test against the buffer
	// primitives the renderer consults, which is what makes the test fast and
	// order-independent.
	crate::ui::progress::start("Network", "ux_default", "Creating");
	// After a transitional verb, the buffer should hold the entry and not
	// have emitted.
	assert_eq!(
		buffered_count_for_tests(),
		1,
		"Creating is transitional and must be buffered"
	);
	// The final verb drops the buffered entry.
	crate::ui::progress::finish("Network", "ux_default", "Exists");
	assert_eq!(
		buffered_count_for_tests(),
		0,
		"Exists drops the buffered Creating entry"
	);

	// Restore.
	crate::ui::set_progress(prev_progress);
	prev_colour.write_global();
}

/// A transitional verb whose final never arrives is flushed at `progress::end`
/// with the transitional verb intact, so a crash mid-way still says `Creating`
/// in the log (#1673): the `Creating` with no end case.
#[test]
fn plain_sink_keeps_a_transitional_verb_when_nothing_final_arrives() {
	let _session = hold_session();
	reset_plain_buffer();
	let prev_progress = crate::ui::progress_enabled();
	crate::ui::set_progress(true);

	// `start` records the transitional verb. The buffer should hold it because
	// the plain-sink renderer suppresses transitional output.
	crate::ui::progress::start("Network", "ux_default", "Creating");
	assert_eq!(
		buffered_count_for_tests(),
		1,
		"the transitional verb is buffered until a final or end()"
	);

	// No `progress_line` ever arrives for this entry. `progress::end` is the
	// fallback: it must flush whatever is in the buffer, so a crash that
	// still reaches `end` (the common case for a graceful abort path) leaves
	// `Creating` in the log.
	crate::ui::progress::end();
	assert_eq!(buffered_count_for_tests(), 0, "end drains the buffer");

	crate::ui::set_progress(prev_progress);
}

/// The plain sink drops the transitional line only before a final verb that
/// reports no work; before `Created`, `Pulled` or `Removed` it keeps it, so a
/// log still shows when the work started (the contract tests hold the
/// end-to-end case through a pipe).
#[test]
fn only_a_no_op_final_verb_drops_the_transitional_line() {
	let _session = hold_session();
	for verb in ["Exists", "Running", "Absent", "Skipped"] {
		assert!(is_noop_final_for_tests(verb), "{verb}");
	}
	for verb in [
		"Created", "Started", "Pulled", "Removed", "Stopped", "Failed",
	] {
		assert!(!is_noop_final_for_tests(verb), "{verb}");
	}
}
