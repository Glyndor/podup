//! Which renderer a recorded event belongs to, and the plain-sink buffer that
//! gives a log one line per resource.
//!
//! The plain sink writes the same line the tree has always written, so there is
//! one line format across both renderers; the buffer is what keeps a log from
//! showing a contradicting pair like `Network x Creating / Network x Exists`.

use std::sync::Mutex;

use super::session::repaint;
use super::Kind;

/// The plain-sink buffer for transitional verbs that have not yet been
/// replaced by a final one.
///
/// A pipe or CI log prints one line per resource, with the final verb. The
/// transitional verb (`Creating`, `Starting`, `Removing`, `Pulling`, …) is
/// buffered until the final one arrives; if no final arrives (a crash mid-way)
/// the buffered verb is flushed at `progress::end` so the log still records
/// what was in flight (#1673).
///
/// Process-global for the same reason the board is: one CLI invocation runs one
/// lifecycle command, and threading a buffer through every engine call site
/// would change 21 signatures to say something none of them decides.
static PLAIN_BUFFER: Mutex<Vec<(Kind, String, String)>> = Mutex::new(Vec::new());

/// Whether `verb` looks transitional: present participle in `-ing`.
///
/// The plain sink buffers these until a final verb arrives for the same
/// `(kind, name)`, so a log shows one line per resource instead of a
/// contradicting pair like `Network x Creating / Network x Exists` (#1673).
///
/// The `-ning` exception matters: `Running` is a final state that ends in
/// `-ing` and would otherwise be buffered, and either flushed at end (which
/// would print it twice: once when the row closes, once when the buffer
/// drains) or held forever (which would lose it on a clean exit). Verbs the
/// call sites actually pass through `progress::start` are the present
/// participles `Creating`, `Starting`, `Stopping`, `Removing`, `Pulling`,
/// `Pushing`, `Recreating`; `Running` reaches the final path through
/// `progress_line`, not `progress::start`.
fn is_transitional(verb: &str) -> bool {
	let verb = verb.trim();
	if verb.len() < 4 {
		return false;
	}
	let lower = verb.to_ascii_lowercase();
	if !lower.ends_with("ing") {
		return false;
	}
	// `Running` and other `-ning` finals are not transitional.
	if lower.ends_with("ning") {
		return false;
	}
	// `Missing` is a noun-form used as a status word, not a transitional.
	lower != "missing"
}

/// Which renderer a just-recorded event belongs to.
pub(super) enum Sink {
	/// A region is open; the event shows up on the next repaint.
	Live,
	/// A board is open but not on a terminal, so the event is a line.
	Plain,
	/// No board at all.
	None,
}

/// Hand a recorded event to whichever renderer owns it.
///
/// The plain sink writes the same line the tree has always written, through
/// [`crate::ui::write_progress_line`] rather than `progress_line`, because going back
/// through the routing that sent the event here would be a loop, and it is what
/// made a piped `up` print nothing at all the first time this was wired up: the
/// board swallowed every line and no renderer put one back.
///
/// A transitional verb on the plain sink is buffered, not written: the log
/// contract is one line per resource with the final verb, and writing the
/// transitional one first would force the writer to overwrite or amend it
/// when the final arrived, which a pipe cannot do. The final verb flushes the
/// buffered one (drops it) and writes itself. `progress::end` flushes any
/// still-buffered entries so a crash mid-way still leaves `Creating` in the
/// log (#1673).
pub(super) fn emit(sink: Sink, kind: Kind, name: &str, verb: &str) {
	match sink {
		Sink::Live => repaint(),
		// Both `Plain` (board open but no region) and `None` (no board at all)
		// go to a log-style line, and the log-style contract is the same in both:
		// one line per resource with the final verb. A `run --rm` that pre-creates
		// networks reaches this branch through `Sink::None` because no board is
		// ever opened, and it must still suppress the transitional verb (#1673).
		Sink::Plain | Sink::None => {
			if !crate::ui::progress_enabled() {
				return;
			}
			if is_transitional(verb) {
				buffer_push(kind, name, verb);
			} else {
				// A final verb that says nothing happened (`Exists`, `Running`,
				// `Absent`, `Skipped`) makes the transitional line before it a
				// contradiction, so it is dropped; a final verb that reports
				// work keeps the transitional line above it, so a log still
				// shows when the work started.
				if let Some(doing) = buffer_take(kind, name) {
					if !is_noop_final(verb) {
						crate::ui::write_progress_line(kind.noun(), name, &doing);
					}
				}
				crate::ui::write_progress_line(kind.noun(), name, verb);
			}
		}
	}
}

/// A final verb after which nothing was done: the resource was already in
/// the state the command wanted, or was not there to act on.
fn is_noop_final(verb: &str) -> bool {
	matches!(verb.trim(), "Exists" | "Running" | "Absent" | "Skipped")
}

/// Remove and return the transitional verb buffered for `(kind, name)`.
fn buffer_take(kind: Kind, name: &str) -> Option<String> {
	let mut buf = PLAIN_BUFFER.lock().ok()?;
	let pos = buf
		.iter()
		.position(|entry| entry.0 == kind && entry.1 == name)?;
	Some(buf.remove(pos).2)
}

/// Append `(kind, name, verb)` to the plain-sink buffer.
fn buffer_push(kind: Kind, name: &str, verb: &str) {
	if let Ok(mut buf) = PLAIN_BUFFER.lock() {
		// A second transitional for the same (kind, name) replaces the first
		// rather than queuing two: a `Recreating` then `Stopping` is reported
		// as the latter, which is the more recent fact.
		buf.retain(|entry| !(entry.0 == kind && entry.1 == name));
		buf.push((kind, name.to_string(), verb.to_string()));
	}
}

/// Drain the buffer in insertion order, emitting one line per entry. Used at
/// `progress::end` to flush transitional verbs that never received a final.
pub(super) fn buffer_drain() {
	let drained: Vec<(Kind, String, String)> = if let Ok(mut buf) = PLAIN_BUFFER.lock() {
		std::mem::take(&mut *buf)
	} else {
		Vec::new()
	};
	for (kind, name, verb) in drained {
		crate::ui::write_progress_line(kind.noun(), &name, &verb);
	}
}

/// Write out every transitional verb the plain sink is still holding. The
/// normal path drains at `end`; a command that fails before reaching it
/// calls this from the error exit, so a log still records what was in
/// flight when it broke.
pub fn flush() {
	buffer_drain();
}

#[cfg(test)]
pub(crate) fn buffer_drain_for_tests() {
	buffer_drain();
}

#[cfg(test)]
pub(crate) fn is_noop_final_for_tests(verb: &str) -> bool {
	is_noop_final(verb)
}

#[cfg(test)]
pub(crate) fn buffered_count_for_tests() -> usize {
	PLAIN_BUFFER.lock().map(|b| b.len()).unwrap_or(0)
}

#[cfg(test)]
#[path = "sink_tests.rs"]
mod tests;
