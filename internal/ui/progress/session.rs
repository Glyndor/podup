//! The board session: what a lifecycle command is working through, and the
//! ticker that keeps the spinner turning while nothing happens.
//!
//! The event surface (`begin`, `start`, `start_anchored`, `finish`, `end`) and
//! the state it acts on live together, because each event is a decision about
//! that one session and reads it under the same lock. Rendering is not here:
//! a recorded event is handed to [`sink`](super::sink), which picks the
//! renderer, and a live repaint is driven by whatever last changed the board.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use super::notes::name_column_width;
use super::sink::{self, Sink};
use super::terminal::{live_terminal, live_width};
use super::{live, row, Board, Kind};

#[cfg(test)]
use super::capture;

/// How often the region repaints while nothing is happening, so the spinner
/// turns during a long pull instead of looking like a hang. The same cadence
/// `terminal::windows`'s resize poll already uses.
const TICK: std::time::Duration = std::time::Duration::from_millis(100);

/// The board for the command currently running, if one asked for it.
///
/// Process-global for the same reason the colour choice is: one CLI invocation
/// runs one lifecycle command, and threading a handle through every engine call
/// site would change 21 signatures to say something none of them decides. A
/// library embedder never gets one; [`begin`] is a no-op unless the CLI turned
/// progress on.
pub(super) static SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// Whether a board is currently open, independent of [`SESSION`]. Set by
/// [`begin`] and cleared by [`end`]; consulted by [`finish`] (which
/// `progress_line` routes through) so the common "no board" path short-circuits
/// the `Mutex::lock` without taking it. Uncontended `Mutex::lock` is ~20 ns but
/// `progress_line` fires once per resource per command, so over a 100-service
/// `up` the lock+unlock pairs add up to ~2 µs of pure syscall work (#1364).
static SESSION_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The repaint ticker, kept so it can be stopped when the board ends.
static TICKER: Mutex<Option<tokio::task::JoinHandle<()>>> = Mutex::new(None);

pub(super) struct Session {
	pub(super) board: Board,
	/// `None` when the sink is plain: not a terminal, colour off, or the
	/// terminal size could not be read.
	pub(super) region: Option<live::Region>,
	pub(super) frame: usize,
	/// Width of the name column, sized from the seeded rows so it does not jump
	/// as rows come and go.
	pub(super) name_width: usize,
	/// Per-row stream tail, kept only while a row is being built. On a terminal
	/// these are the dimmed lines painted under the row; in a pipe they are
	/// written through the plain sink as they arrive. Cleared when the row
	/// finishes: the row itself, plus its closing verb, is the record.
	pub(super) notes: HashMap<(Kind, String), VecDeque<String>>,
}

/// Start a board over `resources`, in the order they will be worked through.
///
/// A no-op when progress output is off, so an embedding crate never gets a
/// board it did not ask for.
pub fn begin(resources: impl IntoIterator<Item = (Kind, String)>) {
	// Recorded before the gate, so a test can read which rows a command asked
	// for without switching the process-wide progress flag on for every other
	// test in the suite. Compiled out of a release build entirely.
	#[cfg(test)]
	let resources: Vec<(Kind, String)> = resources.into_iter().collect();
	#[cfg(test)]
	capture::record_begin(&resources, live_terminal());
	if !crate::ui::progress_enabled() {
		return;
	}
	// Cache the terminal+colour decision here (it cannot change mid-command)
	// and only re-read the width per repaint, so a resize mid-command is
	// still honoured. `live_allowed` previously re-ran both halves ten times a
	// second for the lifetime of the board (#1364).
	let live = live_terminal();
	let board = Board::new(resources);
	let name_width = board
		.live_rows()
		.iter()
		.fold(0, |width, r| name_column_width(width, &r.name));
	let region = live.then(|| live::Region::new(live::Target::Stderr));
	if let Ok(mut slot) = SESSION.lock() {
		*slot = Some(Session {
			board,
			region,
			frame: 0,
			name_width,
			notes: HashMap::new(),
		});
	}
	// The flag goes up *after* the session is installed: a `finish` racing the
	// install sees either no session (`SESSION_OPEN` false → no lock taken) or
	// a fully-built session. The reverse order would let `finish` take the
	// lock only to find an empty session and return false: same result, more
	// work.
	SESSION_OPEN.store(true, std::sync::atomic::Ordering::Release);
	if live {
		spawn_ticker();
	}
	repaint();
}

/// Report that work on a resource has begun.
///
/// This is the event the tree could not previously produce: every one of the 21
/// existing progress sites fires once the work is already over.
pub fn start(kind: &str, name: &str, verb: &str) {
	start_anchored(kind, name, verb, None, None);
}

/// As [`start`], but insert the row in front of `anchor_kind, anchor_name`
/// rather than appending it. The `up` build path uses this so the image row
/// sits before the container rows that depend on it; a plain `start` would
/// put it after them, and the reader would see the container starting before
/// the row that says which image is being built.
///
/// `None` for either anchor argument falls back to plain `start` semantics.
pub fn start_anchored(
	kind: &str,
	name: &str,
	verb: &str,
	anchor_kind: Option<&str>,
	anchor_name: Option<&str>,
) {
	let Some(kind) = Kind::from_noun(kind) else {
		return;
	};
	// Recorded before the sink decision so a test can read what an engine path
	// asked the board to render, including the transitions the plain sink would
	// have suppressed in production. Same scope as `record_begin`/`record_end`:
	// test-only, compiled out of a release build entirely.
	#[cfg(test)]
	capture::record_start(kind, name, verb);
	let anchor = match (anchor_kind, anchor_name) {
		(Some(k), Some(n)) => Kind::from_noun(k).map(|ak| (ak, n.to_string())),
		_ => None,
	};
	let sink = {
		let Ok(mut slot) = SESSION.lock() else {
			return;
		};
		match slot.as_mut() {
			Some(session) => {
				if let Some((anchor_kind, anchor_name)) = anchor {
					session.board.start_before(
						anchor_kind,
						&anchor_name,
						kind,
						name,
						verb,
						Instant::now(),
					);
				} else {
					session.board.start(kind, name, verb, Instant::now());
				}
				// A row that was not seeded (the image row `up` inserts for a
				// missing image, #1684) can be wider than the column `begin`
				// sized from the seeded names; without this it rendered as
				// `localhost/u…` while every seeded row fit (#1700).
				session.name_width = name_column_width(session.name_width, name);
				if session.region.is_some() {
					Sink::Live
				} else {
					Sink::Plain
				}
			}
			None => Sink::None,
		}
	};
	sink::emit(sink, kind, name, verb);
}

/// Report that work on a resource has finished. Always returns `true` once
/// the event has been emitted (or buffered); the caller must not print its own
/// line in that case. Returns `false` only for an unrecognised `kind`, which
/// the caller treats as "no event happened" (#1673: the plain-sink buffer
/// logic used to live partly here and partly in `ui::progress_line`, and a
/// `Sink::None` finish call dropped the buffer entry on the floor).
pub(in crate::ui) fn finish(kind: &str, name: &str, verb: &str) -> bool {
	let Some(kind) = Kind::from_noun(kind) else {
		return false;
	};
	// Recorded before the session gate for the same reason `record_begin` and
	// `record_start` are: a test that wants the row lifecycle end-to-end sees
	// every event, even when no board was actually opened (which is what
	// `PROGRESS_ENABLED=false` and `cargo test` produce). The session gate is
	// about who renders the event, not about whether the event happened.
	#[cfg(test)]
	capture::record_finish(kind, name, verb);
	// Short-circuit the common "no board open" path before taking the lock
	// (#1364). `progress_line` is the hottest UI site: a 100-service `up`
	// fires it 100 times, and every call would otherwise acquire and release
	// the global `SESSION` mutex just to discover there is no session. With
	// the flag, the lock is only taken when `begin` was actually called.
	let sink = if !SESSION_OPEN.load(std::sync::atomic::Ordering::Acquire) {
		Sink::None
	} else {
		let Ok(mut slot) = SESSION.lock() else {
			return false;
		};
		match slot.as_mut() {
			Some(session) => {
				session.board.finish(kind, name, verb, Instant::now());
				// Drop the per-row stream tail: the row is closing, and the
				// dimmed lines under it would otherwise linger as the next
				// repaint's noise. A finished row keeps no shadow.
				session.notes.remove(&(kind, name.to_string()));
				if session.region.is_some() {
					Sink::Live
				} else {
					Sink::Plain
				}
			}
			None => Sink::None,
		}
	};
	// `Sink::None` means there is no board; still go through `emit` so the
	// plain-sink buffer logic runs in the no-board path too. `emit` decides
	// what to actually emit: a final verb goes to stderr immediately, a
	// transitional verb goes to the buffer to be flushed at `end`.
	sink::emit(sink, kind, name, verb);
	true
}

/// Close the board: stop the ticker, restore the cursor.
///
/// Idempotent, because the commands that open a board have several exits.
pub fn end() {
	#[cfg(test)]
	capture::record_end();
	if !SESSION_OPEN.swap(false, std::sync::atomic::Ordering::AcqRel) {
		// No board was ever opened; the flag is the single source of truth
		// for that. Drop it first so a racing `finish` doesn't take the lock
		// just to find an empty session (#1364).
		sink::buffer_drain();
		return;
	}
	if let Ok(mut slot) = TICKER.lock() {
		if let Some(handle) = slot.take() {
			handle.abort();
		}
	}
	let Ok(mut slot) = SESSION.lock() else {
		sink::buffer_drain();
		return;
	};
	if let Some(session) = slot.take() {
		// Walk the cursor back over the live region so the trailing rows are
		// not left dangling, then erase from there. The rows that were in
		// the live region stay where they are on screen: they are the
		// permanent record at this point, no different from scrollback. A
		// re-paint here would write the same bytes again, and a `script`
		// capture would show the row twice (once written, once erased, once
		// written again); keeping the cursor move without a re-write makes
		// the row appear exactly once in the capture (#1675).
		if let Some(region) = session.region.as_ref() {
			region.close_out();
		}
		// Drop every per-row stream tail. The session is gone; the VecDeques
		// go with it. Belt-and-braces: `finish` already clears each row's
		// notes as it closes, so anything still in the map here belongs to a
		// row that closed without a `finish`, and would otherwise linger as
		// garbage the next test picks up.
		drop(session.notes);
	}
	// Flush any buffered transitional verbs that never received a final one.
	// The plain sink is the only one that buffers, but `end` is the right place
	// to drain whatever the board left behind (#1673).
	sink::buffer_drain();
}

/// Repaint the region, if there is one. A plain sink does nothing here: its
/// lines are emitted by the event calls themselves.
pub(super) fn repaint() {
	let Ok(mut slot) = SESSION.lock() else {
		return;
	};
	let Some(session) = slot.as_mut() else {
		return;
	};
	let frame = session.frame;
	let name_width = session.name_width;
	let Session {
		board,
		region,
		notes,
		..
	} = session;
	let Some(region) = region.as_mut() else {
		return;
	};
	// Re-read the width every repaint, so a resize mid-command does not leave
	// every later line wrapping, and wrapping is what breaks the arithmetic.
	// Only the width is re-read; the terminal+colour decision is cached at
	// `begin` (#1364).
	let width = live_width().unwrap_or(0);
	let now = Instant::now();
	let scrollback: Vec<String> = board
		.take_completed_prefix()
		.iter()
		.map(|r| row::render(r, name_width, frame, now, width))
		.collect();
	let mut lines = Vec::new();
	let rows = board.live_rows();
	if !rows.is_empty() {
		let (done, total) = board.tally();
		lines.push(row::summary(done, total, width));
		for r in rows {
			lines.push(row::render(r, name_width, frame, now, width));
			// Per-row tail: the last few stream lines the row has produced,
			// rendered dimmed and indented one space so they sit under the
			// row without competing with the marker column on the next row.
			if let Some(tail) = notes.get(&(r.kind, r.name.clone())) {
				for line in tail.iter() {
					lines.push(row::render_note(line, width));
				}
			}
		}
	}
	region.show(&scrollback, &lines);
}

/// Advance the spinner and repaint, so a long pull turns rather than freezing.
fn spawn_ticker() {
	let handle = tokio::spawn(async {
		let mut interval = tokio::time::interval(TICK);
		loop {
			interval.tick().await;
			{
				let Ok(mut slot) = SESSION.lock() else { return };
				match slot.as_mut() {
					Some(session) => session.frame = session.frame.wrapping_add(1),
					None => return,
				}
			}
			repaint();
		}
	});
	if let Ok(mut slot) = TICKER.lock() {
		*slot = Some(handle);
	}
}

#[cfg(test)]
pub(crate) fn name_width_for_tests() -> usize {
	SESSION
		.lock()
		.ok()
		.and_then(|slot| slot.as_ref().map(|s| s.name_width))
		.unwrap_or(0)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
