//! Whether output carries ANSI codes at all.
//!
//! One process-wide colour choice, resolved against the TTY state of the stream
//! being written and against `NO_COLOR`. Every sink asks here rather than
//! deciding for itself, which is what keeps `--ansi`, `NO_COLOR` and a
//! redirected stdout from disagreeing between two commands.
//!
//! `set_color_choice` also constructs the auto streams once so anstream enables
//! virtual-terminal processing on Windows for the rest of the process.

use std::io::IsTerminal;

use anstream::ColorChoice;

/// Apply the resolved colour choice process-wide and enable Windows VT once.
///
/// anstream's `stdout()`/`stderr()` then honour this choice together with
/// `NO_COLOR`/`CLICOLOR` and TTY detection.
pub fn set_color_choice(choice: ColorChoice) {
	choice.write_global();
	// Constructing the auto streams enables virtual-terminal processing on
	// Windows so raw-sink callers can emit ANSI safely; a no-op elsewhere.
	let _ = anstream::AutoStream::auto(std::io::stdout());
	let _ = anstream::AutoStream::auto(std::io::stderr());
}

/// `NO_COLOR` is set to a non-empty value (the no-color.org convention).
fn no_color() -> bool {
	std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

/// Whether a stream that is (or isn't) a TTY should be coloured under `choice`.
/// `Auto` defers to the TTY and `NO_COLOR`. Pure (takes the choice as an argument)
/// so the policy is tested without mutating the process-global choice.
fn colored_with(choice: ColorChoice, is_terminal: bool) -> bool {
	match choice {
		ColorChoice::Always | ColorChoice::AlwaysAnsi => true,
		ColorChoice::Never => false,
		ColorChoice::Auto => is_terminal && !no_color(),
	}
}

/// Whether a stream that is (or isn't) a TTY should be coloured under the global
/// choice.
fn colored(is_terminal: bool) -> bool {
	colored_with(ColorChoice::global(), is_terminal)
}

/// Whether a raw (non-anstream) write to stdout should embed ANSI codes.
pub fn stdout_colored() -> bool {
	colored(std::io::stdout().is_terminal())
}

/// Whether a raw (non-anstream) write to stderr should embed ANSI codes.
pub fn stderr_colored() -> bool {
	colored(std::io::stderr().is_terminal())
}

#[cfg(test)]
#[path = "colour_tests.rs"]
mod tests;
