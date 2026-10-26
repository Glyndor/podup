//! The stream tail kept under each row while it is being built, and the name
//! column that sizes the rows above it.
//!
//! On a terminal these are dimmed lines painted under the row; in a pipe the
//! same lines are written through the plain sink as they arrive. One entry per
//! line is the invariant [`Region`](super::super::live::Region) rests on: it counts one
//! terminal row per entry, and an entry holding an embedded newline breaks the
//! arithmetic for every repaint after it (#1733).

use std::collections::HashMap;
use std::collections::VecDeque;

use super::super::session::{repaint, SESSION};
use super::super::sink::Sink;
use super::super::Kind;

/// How many stream lines a row keeps under itself when it is being built.
/// Tuned by reading a real buildah stream: the last few lines are the layer
/// hash, the `--> running step` markers and the `COMMIT` line, which is the
/// shape a reader needs to tell where the build is. Five or more drowns the
/// row; three loses the layer hash before the next step lands; four is the
/// narrowest window that keeps both.
const MAX_NOTES_PER_ROW: usize = 4;

/// The lines one stream chunk contributes to a row's tail.
///
/// A builder writes several lines in one chunk, and the repaint arithmetic
/// counts entries: a note holding `\n` is drawn as several terminal rows and
/// counted as one, so `cursor_up(painted)` lands short and every later
/// repaint erases too few lines (#1733). Splitting here keeps the invariant
/// `Region` documents, that one entry is one row.
///
/// `split('\n')` rather than `lines()` on purpose: `lines()` also swallows a
/// trailing `\r`, and the `trim()` here handles `\r\n` anyway, so the two
/// responsibilities stay apart. Empty parts are dropped, matching what a
/// single blank line already did.
fn note_lines(line: &str) -> impl Iterator<Item = &str> {
	line.split('\n')
		.map(str::trim)
		.filter(|part| !part.is_empty())
}

/// Test hook for [`note_lines`], which is otherwise only reachable through a
/// live region the cargo-test runner cannot give us.
#[cfg(test)]
pub(crate) fn note_lines_for_tests(line: &str) -> Vec<&str> {
	note_lines(line).collect()
}

/// Hand one line of a row's stream output to the renderer.
///
/// On a live terminal the line is appended to the per-row tail kept under the
/// row (capped at `MAX_NOTES_PER_ROW`), then the region is repainted so the
/// dimmed line shows under the row. In a pipe or whenever the session has no
/// region the same call writes `<name> | <line>` to stderr directly, so a
/// redirected stderr still sees every stream line, prefixed the way the
/// `logs` command prefixes container output.
///
/// The transition is decided once, here, by the same predicate `start` uses:
/// `live_terminal()` is consulted at `begin` and the result is cached, so a
/// live session always has a region, and a non-live session never does. That
/// keeps the decision in lockstep with the rest of the renderer; a divergent
/// decision here would let a live run leak stream lines to stderr and a
/// piped run paint dimmed notes on a sink that does not exist.
///
/// Empty lines and lines that trim to empty are skipped: a buildah stream that
/// emits `\n` between blocks would otherwise fill the tail with blanks and
/// push the meaningful lines off the end.
pub fn note_for(kind: &str, name: &str, line: &str) {
	let Some(kind) = Kind::from_noun(kind) else {
		return;
	};
	// One entry per line. A builder stream chunk can carry several lines in a
	// single write, and the repaint arithmetic counts entries: a note holding
	// `\n` is drawn as several terminal rows but counted as one, so from that
	// frame on `cursor_up(painted)` lands short and every repaint erases too
	// few lines. That is the eighty-times-scrolled board in #1733, and
	// `Region`'s own doc comment predicts it: it says a line that wraps makes
	// the terminal count two rows where this counted one. An embedded newline
	// breaks the same invariant by a different route, and the width fit does
	// not cover it because `chars().take(width)` counts `\n` as an ordinary
	// character.
	//
	// It also restores `MAX_NOTES_PER_ROW`, which a single arbitrarily tall
	// entry defeats.
	//
	// `split('\n')` rather than `lines()` on purpose: `lines()` also swallows
	// a trailing `\r`, and the `trim()` below handles `\r\n` anyway, so this
	// keeps the two responsibilities apart.
	for trimmed in note_lines(line) {
		let sink = {
			let Ok(mut slot) = SESSION.lock() else {
				return;
			};
			match slot.as_mut() {
				Some(session) => {
					if session.region.is_some() {
						push_note_live(&mut session.notes, kind, name, trimmed);
						Sink::Live
					} else {
						Sink::Plain
					}
				}
				None => Sink::None,
			}
		};
		match sink {
			Sink::Live => repaint(),
			Sink::Plain | Sink::None => {
				if !crate::ui::progress_enabled() {
					return;
				}
				use std::io::Write;
				let _ = writeln!(anstream::stderr(), "{name} | {trimmed}");
			}
		}
	}
}

/// Push one line into a row's tail, keeping only the last [`MAX_NOTES_PER_ROW`]
/// lines. Split out of [`note_for`] so the trim/append/pop-front logic is
/// reachable from a test without first opening a live region, which the
/// cargo-test runner cannot give us.
fn push_note_live(
	notes: &mut HashMap<(Kind, String), VecDeque<String>>,
	kind: Kind,
	name: &str,
	trimmed: &str,
) {
	let tail = notes.entry((kind, name.to_string())).or_default();
	tail.push_back(trimmed.to_string());
	while tail.len() > MAX_NOTES_PER_ROW {
		tail.pop_front();
	}
}

#[cfg(test)]
pub(crate) const MAX_NOTES_PER_ROW_FOR_TESTS: usize = MAX_NOTES_PER_ROW;

/// The name column is as wide as the widest name it has seen, never under
/// 12 or over 40 cells; a name past 40 is cut with an ellipsis by the row
/// renderer. Applied at `begin` over the seeded rows and again for every row
/// `start` adds later.
pub(in super::super) fn name_column_width(current: usize, name: &str) -> usize {
	current.max(name.chars().count()).clamp(12, 40)
}

#[cfg(test)]
pub(crate) fn push_note_live_for_tests(
	notes: &mut HashMap<(Kind, String), VecDeque<String>>,
	kind: Kind,
	name: &str,
	trimmed: &str,
) {
	push_note_live(notes, kind, name, trimmed);
}

#[cfg(test)]
#[path = "notes_tests.rs"]
mod tests;
