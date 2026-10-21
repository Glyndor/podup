//! Probing the terminal the board draws on: whether a live region is allowed at
//! all, and how wide it is right now.
//!
//! Separate from the session that asks, because both answers are per-repaint
//! properties of the terminal rather than of the command.

/// Whether the live tail region is allowed at all.
///
/// The decision depends on the terminal only: the spinner and the in-place
/// repaint are animations the terminal drives, and a log file is no terminal.
/// `is_terminal()` answers that without consulting the colour choice, so
/// NO_COLOR=1 and `--ansi never` no longer collapse the live board into a
/// stream of plain lines (#1672): a styled run that asked for no escapes keeps
/// its animation, and the styling is stripped at the renderer instead of the
/// renderer being skipped.
///
/// `crate::ui::stderr_colored()` is consulted separately, inside
/// [`row::render`](super::row::render), so a single board colour can be on or off
/// without the region being on or off.
pub(super) fn live_terminal() -> bool {
	use std::io::IsTerminal;
	std::io::stderr().is_terminal()
}

/// The current terminal width from a `window_size()` answer, or `None` when
/// the runtime cannot tell. Per-repaint so a resize mid-command is reflected
/// on the next frame (#1364).
pub(super) fn live_width() -> Option<usize> {
	width_from(crate::engine::query::terminal::window_size())
}

/// The usable width from a `window_size()` answer.
///
/// Split out so the one thing that can silently be wrong here is testable:
/// `window_size` returns **(rows, cols)**, in that order, and reading the
/// first element as the width truncated every board line to the terminal's
/// *height* instead. Measured on a 100x30 pty, every verb came out as `Pen…`,
/// and nothing in the type system or the suite objected.
fn width_from(size: Option<(u16, u16)>) -> Option<usize> {
	size.map(|(_rows, cols)| cols as usize)
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
