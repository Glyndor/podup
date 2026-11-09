//! The one place that writes to stdout and stderr.
//!
//! A direct `println!` panics when the write fails, and Rust ignores SIGPIPE,
//! so a reader that closed the pipe early (`podup ps | head -1`) turned into
//! a panic that the hook in `main` had to catch afterwards. The macros here
//! decide what a failed write means instead, and the decision lives in one
//! place: a closed pipe ends the process quietly, with the status a Unix tool
//! returns when its reader is gone, and every other failure is handed back to
//! the caller so the command can fail instead of losing its output.
//!
//! The macros are how the rest of the code writes. The functions are what they
//! expand to, and they are public because a macro expands in the crate that
//! calls it.
//!
//! A writer that builds its own text (the table, the bold header, the labelled
//! lines, the rendered config) does not call a macro, but it reaches the same
//! decision through [`settle`], so no path out of the binary can lose a row
//! without saying so.

use std::fmt::Arguments;
use std::io::{self, Write};
use std::process;

/// Write `args` to stdout.
pub fn write_out(args: Arguments) -> io::Result<()> {
	let mut handle = io::stdout().lock();
	settle(write_to(&mut handle, args, false))
}

/// Write `args` and a newline to stdout.
pub fn write_outln(args: Arguments) -> io::Result<()> {
	let mut handle = io::stdout().lock();
	settle(write_to(&mut handle, args, true))
}

/// Write `args` to stderr.
pub fn write_err(args: Arguments) -> io::Result<()> {
	let mut handle = io::stderr().lock();
	settle(write_to(&mut handle, args, false))
}

/// Write `args` and a newline to stderr.
pub fn write_errln(args: Arguments) -> io::Result<()> {
	let mut handle = io::stderr().lock();
	settle(write_to(&mut handle, args, true))
}

/// Write `args` to `handle`, with a newline when one is asked for.
///
/// The text and the newline are two writes, and the handle is a line-buffered
/// one, so the line still reaches the pipe in a single flush.
fn write_to(handle: &mut impl Write, args: Arguments, newline: bool) -> io::Result<()> {
	let mut written = handle.write_fmt(args);
	if written.is_ok() && newline {
		written = handle.write_all(b"\n");
	}
	written
}

/// Turn the result of a write into the outcome the caller expects.
///
/// The one place that decides what a failed write means: a reader that closed
/// the pipe ends the process quietly, and every other failure is handed back
/// to the caller so the command can fail instead of losing its output.
///
/// Public because the writers that build their own text rather than calling
/// the macros reach the same decision: the table, the bold header, the
/// labelled lines and the rendered config. Each of them writes bytes of its
/// own, and each of them used to drop the failure on the floor.
pub fn settle(result: io::Result<()>) -> io::Result<()> {
	match result {
		Ok(()) => Ok(()),
		Err(e) if reader_is_gone(&e) => end_quietly(),
		Err(e) => Err(e),
	}
}

/// Whether the failure means the reader of the pipe is gone.
///
/// The reader of `podup ps | head -1` closed its end, and the write that
/// follows fails with this kind. Nothing else means that.
fn reader_is_gone(error: &io::Error) -> bool {
	error.kind() == io::ErrorKind::BrokenPipe
}

/// End the process the way a Unix tool ends when its reader closed the pipe.
///
/// The reader of `podup ps | head -1` is gone, so there is nothing left to
/// say to it and the command has nothing left to do. The status is success:
/// the work was done, and nobody is left to read the rest of it. The panic
/// hook in `main` ends the process the same way for a write that was not
/// routed through here.
fn end_quietly() -> ! {
	process::exit(0)
}

/// Write to stdout, and report whether the write succeeded.
///
/// The value is the write's result, so a caller that can fail does
/// `podup::ui::out!("...")?`.
#[macro_export]
macro_rules! out {
	() => {
		$crate::ui::print::write_out(format_args!(""))
	};
	($($arg:tt)*) => {
		$crate::ui::print::write_out(format_args!($($arg)*))
	};
}

/// Write a line to stdout, and report whether the write succeeded.
///
/// The value is the write's result, so a caller that can fail does
/// `podup::ui::outln!("...")?`.
#[macro_export]
macro_rules! outln {
	() => {
		$crate::ui::print::write_outln(format_args!(""))
	};
	($($arg:tt)*) => {
		$crate::ui::print::write_outln(format_args!($($arg)*))
	};
}

/// Write to stderr, and report whether the write succeeded.
///
/// The value is the write's result, so a caller that can fail does
/// `podup::ui::err!("...")?`.
#[macro_export]
macro_rules! err {
	() => {
		$crate::ui::print::write_err(format_args!(""))
	};
	($($arg:tt)*) => {
		$crate::ui::print::write_err(format_args!($($arg)*))
	};
}

/// Write a line to stderr, and report whether the write succeeded.
///
/// The value is the write's result, so a caller that can fail does
/// `podup::ui::errln!("...")?`.
#[macro_export]
macro_rules! errln {
	() => {
		$crate::ui::print::write_errln(format_args!(""))
	};
	($($arg:tt)*) => {
		$crate::ui::print::write_errln(format_args!($($arg)*))
	};
}

#[cfg(test)]
#[path = "print_tests.rs"]
mod tests;
