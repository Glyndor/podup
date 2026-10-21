//! The live board: what a lifecycle command is working through, and where it
//! has got to.
//!
//! One model, two renderers. Which one runs is decided from the terminal and the
//! colour choice, never from the command: `up` on a tty repaints a tail region,
//! `up` in a pipe emits the same events as plain append-only lines. **Animation
//! in a CI log is a defect**, and so is a CI log that says less than the
//! terminal did: both renderers see every event, including the intermediate
//! transitions the old output dropped entirely.
//!
//! Everything goes to stderr. stdout stays a clean pipe, which is what lets
//! `run -d` keep printing its container id there.

mod board;
#[cfg(test)]
pub(crate) mod capture;
mod live;
mod notes;
mod row;
mod session;
mod sink;
mod terminal;
#[cfg(test)]
#[path = "test_support.rs"]
mod test_support;

pub use board::{Board, Kind, Row, State};
pub use live::{Region, Target};
pub use notes::note_for;
pub use session::{begin, end, start, start_anchored};
pub use sink::flush;

pub(super) use session::finish;

#[cfg(test)]
pub(crate) use notes::{
	note_lines_for_tests, push_note_live_for_tests, MAX_NOTES_PER_ROW_FOR_TESTS,
};
#[cfg(test)]
pub(crate) use session::name_width_for_tests;
#[cfg(test)]
pub(crate) use sink::{buffer_drain_for_tests, buffered_count_for_tests, is_noop_final_for_tests};
