//! The console-API implementation of the terminal contract — see the module
//! doc in `mod.rs` for the contract and the restore-on-drop invariant.
//!
//! Windows has no termios: raw mode is a console *mode* cleared on the stdin
//! handle (line input, echo, Ctrl-C processing) plus virtual-terminal flags on
//! both ends, so keystrokes arrive as VT byte sequences and the remote pty's
//! VT output renders instead of printing as garbage. And it has no `SIGWINCH`:
//! window changes are found by polling the screen-buffer size, which is cheap
//! enough at four times a second to be imperceptible and avoids competing with
//! the stdin pump for console input events.

// The console mode and screen-buffer calls are Win32 FFI. The crate denies
// `unsafe` and modules that need it opt back in locally, with a soundness
// comment per block — see `engine::lock` and `engine::staging` for the same
// pattern.
#![allow(unsafe_code)]

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::{
	GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle, SetConsoleMode, CONSOLE_MODE,
	CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
	ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_ERROR_HANDLE,
	STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};

/// A console switched to raw mode, restored on drop.
///
/// Holding the *original* modes rather than reconstructing "sane" ones
/// matters: the caller's shell may run with its own console flags, and putting
/// the console into what podup thinks is normal would quietly change them.
/// Both ends are touched — stdin so keystrokes stop being line-buffered and
/// echoed, stdout so the VT sequences a pty emits are interpreted — so both
/// originals are held and both are restored.
pub(crate) struct RawMode {
	stdin: HANDLE,
	stdin_original: CONSOLE_MODE,
	stdout: HANDLE,
	stdout_original: CONSOLE_MODE,
}

// SAFETY: the standard console handles are process-global pseudo-handles, not
// tied to the thread that retrieved them; `SetConsoleMode` is documented as
// callable from any thread. The raw pointers inside `HANDLE` are what stops
// the auto-impl, not any real thread affinity.
unsafe impl Send for RawMode {}

impl RawMode {
	/// Switch the console to raw mode, or return `None` when stdin or stdout
	/// is not a console.
	///
	/// Not being a console is the ordinary case for `podup exec` in a script
	/// or a pipeline, not an error: there is no line discipline to disable,
	/// and the caller streams bytes as before.
	pub(crate) fn enable() -> Option<Self> {
		// SAFETY: `GetStdHandle` takes no pointers and only returns a handle;
		// a missing standard handle comes back null or invalid, checked below.
		let stdin = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
		let stdout = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
		Self::enable_on(stdin, stdout)
	}

	/// The same, on explicit handles.
	///
	/// `enable` is the only place that consults the ambient handles, which
	/// keeps this testable: a test that asserted on `enable()` directly would
	/// be asserting on whether the test runner has a console — and on the way
	/// to failing it would put that console into raw mode.
	pub(super) fn enable_on(stdin: HANDLE, stdout: HANDLE) -> Option<Self> {
		if stdin.is_null() || stdin == INVALID_HANDLE_VALUE {
			return None;
		}
		if stdout.is_null() || stdout == INVALID_HANDLE_VALUE {
			return None;
		}

		let mut stdin_original: CONSOLE_MODE = 0;
		// SAFETY: `stdin_original` is a correctly sized mode owned here, and
		// `GetConsoleMode` only writes into it. A non-console handle fails the
		// call rather than misbehaving — that is exactly the "not a terminal"
		// answer.
		if unsafe { GetConsoleMode(stdin, &mut stdin_original) } == 0 {
			return None;
		}
		let mut stdout_original: CONSOLE_MODE = 0;
		// SAFETY: as above, for the output handle.
		if unsafe { GetConsoleMode(stdout, &mut stdout_original) } == 0 {
			return None;
		}

		// Line input and echo are the console's line discipline; processed
		// input turns Ctrl-C into an event instead of a byte. All three must
		// go so keystrokes — including Ctrl-C — travel to the remote command,
		// which is what raw mode means. Virtual-terminal input makes arrows
		// and function keys arrive as the VT sequences the remote pty expects.
		let stdin_raw = (stdin_original
			& !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT))
			| ENABLE_VIRTUAL_TERMINAL_INPUT;
		// SAFETY: the handle is a console (its mode was just read) and the
		// mode is a plain flag word derived from the current one.
		if unsafe { SetConsoleMode(stdin, stdin_raw) } == 0 {
			return None;
		}

		// Added to the current flags, never replacing them: anstream may have
		// already enabled VT processing for colour output, and clobbering the
		// rest of the mode word would undo whatever else the shell had set.
		let stdout_raw = stdout_original | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
		// SAFETY: as above, for the output handle.
		if unsafe { SetConsoleMode(stdout, stdout_raw) } == 0 {
			// stdin is already raw; put it back before reporting "no console"
			// so a failed enable never half-changes the caller's terminal.
			// SAFETY: restoring the exact mode read from this handle above.
			unsafe { SetConsoleMode(stdin, stdin_original) };
			return None;
		}

		Some(Self {
			stdin,
			stdin_original,
			stdout,
			stdout_original,
		})
	}
}

impl Drop for RawMode {
	fn drop(&mut self) {
		// SAFETY: these are the exact modes read from these same handles in
		// `enable_on`, so restoring them cannot put the console in a state it
		// was not already in. Errors are ignored because there is nothing
		// useful to do while unwinding, and reporting one would replace the
		// real error the caller is already handling.
		unsafe {
			SetConsoleMode(self.stdin, self.stdin_original);
			SetConsoleMode(self.stdout, self.stdout_original);
		}
	}
}

/// The console window's current size as `(rows, cols)`, or `None` when no
/// handle can answer.
///
/// Used to size the remote pty at start and to follow it on window changes:
/// without this, a full-screen program inside the container draws to an 80x24
/// default and redraws wrong the moment the window changes.
///
/// **stdout first, then stderr** — not stdin, unlike Unix: the screen buffer
/// is an output-side object, and the input handle does not answer
/// `GetConsoleScreenBufferInfo`. stderr covers a redirected stdout, the same
/// reverse case the Unix fallback covers.
pub(crate) fn window_size() -> Option<(u16, u16)> {
	// SAFETY: `GetStdHandle` takes no pointers; `size_of` validates what it
	// returns.
	let stdout = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
	let stderr = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
	size_of(stdout).or_else(|| size_of(stderr))
}

/// `GetConsoleScreenBufferInfo` on one handle.
fn size_of(handle: HANDLE) -> Option<(u16, u16)> {
	if handle.is_null() || handle == INVALID_HANDLE_VALUE {
		return None;
	}
	// SAFETY: `info` is a correctly sized, zeroed struct owned here, and the
	// call only writes into it. A non-console handle fails the call rather
	// than writing garbage.
	let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
	if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } != 0 {
		return window_extent(&info);
	}
	None
}

/// The *visible window* extent from a screen-buffer report, as `(rows, cols)`.
///
/// `srWindow`, not `dwSize`: the buffer runs thousands of lines of scrollback,
/// and sizing the remote pty to it would have a full-screen program paint a
/// frame taller than the screen. The rectangle is inclusive on both ends,
/// hence the `+ 1`s. Pure, so the arithmetic — the part worth pinning — is
/// testable without a console.
fn window_extent(info: &CONSOLE_SCREEN_BUFFER_INFO) -> Option<(u16, u16)> {
	let rows = i32::from(info.srWindow.Bottom) - i32::from(info.srWindow.Top) + 1;
	let cols = i32::from(info.srWindow.Right) - i32::from(info.srWindow.Left) + 1;
	// A degenerate or inverted rectangle has no usable geometry — treat it as
	// unknown rather than sizing the remote pty to nothing.
	if rows <= 0 || cols <= 0 {
		return None;
	}
	Some((rows as u16, cols as u16))
}

/// Resize events for the interactive pump: yields a size to apply whenever the
/// caller's window changes.
///
/// The console has no resize signal, so this polls [`window_size`] four times
/// a second and reports only changes. The alternative — `ReadConsoleInput`
/// watching `WINDOW_BUFFER_SIZE_EVENT` — is event-driven but competes with the
/// stdin byte pump for the same console handle; polling costs one cheap call
/// per tick and touches nothing the pump owns.
pub(crate) struct ResizeWatcher {
	interval: tokio::time::Interval,
	last: Option<(u16, u16)>,
}

impl ResizeWatcher {
	/// Start watching, from the current size.
	///
	/// `Result` for signature parity with the Unix watcher, whose signal
	/// registration can genuinely fail; this one cannot.
	pub(crate) fn new() -> std::io::Result<Self> {
		let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
		// A pump busy streaming output can miss ticks; catching up in a burst
		// would report the same size several times for no reason.
		interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
		Ok(Self {
			interval,
			last: window_size(),
		})
	}

	/// The next size to apply: pends until a poll observes a change.
	pub(crate) async fn next(&mut self) -> (u16, u16) {
		loop {
			self.interval.tick().await;
			if let Some(size) = resize_due(&mut self.last, window_size()) {
				return size;
			}
		}
	}
}

/// Whether a polled size warrants a resize: `Some` only when it is readable
/// and differs from the last one reported, recording it as reported. Pure so
/// the dedup — the part that decides whether the pump spams resize calls —
/// is testable without a console.
fn resize_due(last: &mut Option<(u16, u16)>, current: Option<(u16, u16)>) -> Option<(u16, u16)> {
	let current = current?;
	if *last == Some(current) {
		return None;
	}
	*last = Some(current);
	Some(current)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A non-console handle is declined rather than failing — the path
	/// `podup exec` takes inside a pipeline, which must keep working.
	///
	/// Asked of an explicit `NUL` handle rather than of ambient stdin: the
	/// same assertion on `enable()` would be testing how the harness was
	/// invoked, and on the way to failing it would put a real console into
	/// raw mode.
	#[test]
	fn a_non_console_handle_is_declined() {
		use std::os::windows::io::AsRawHandle;
		let devnull = std::fs::File::open("NUL").expect("NUL opens");
		let handle = devnull.as_raw_handle() as HANDLE;
		assert!(
			RawMode::enable_on(handle, handle).is_none(),
			"a non-console handle must not be switched to raw mode"
		);
	}

	/// Likewise the size query: absence is a valid answer, not an error.
	#[test]
	fn a_non_console_handle_has_no_size() {
		use std::os::windows::io::AsRawHandle;
		let devnull = std::fs::File::open("NUL").expect("NUL opens");
		assert_eq!(size_of(devnull.as_raw_handle() as HANDLE), None);
	}

	/// The window extent comes from `srWindow` and is inclusive on both ends;
	/// `dwSize` (the scrollback buffer) must play no part in it.
	#[test]
	fn the_window_extent_is_the_visible_rectangle() {
		let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
		info.dwSize.X = 120;
		info.dwSize.Y = 9001; // scrollback: must not leak into the answer
		info.srWindow.Left = 0;
		info.srWindow.Right = 119;
		info.srWindow.Top = 8971;
		info.srWindow.Bottom = 9000;
		assert_eq!(window_extent(&info), Some((30, 120)));
	}

	/// A degenerate rectangle is unknown geometry, not a 0x0 pty.
	#[test]
	fn a_degenerate_window_has_no_size() {
		// A zeroed rectangle is a legal 1x1 window, so degeneracy has to be
		// forced: an inverted extent (Right < Left) yields a non-positive
		// width.
		let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
		info.srWindow.Right = -2;
		assert_eq!(window_extent(&info), None);
	}

	/// The poll dedup: only a *changed*, readable size is worth a resize call.
	#[test]
	fn resize_is_due_only_on_a_changed_readable_size() {
		let mut last = Some((24, 80));
		// Unchanged: nothing to do.
		assert_eq!(resize_due(&mut last, Some((24, 80))), None);
		// Unreadable (window lost): nothing to apply, and the last size is
		// kept so regaining the same one stays quiet.
		assert_eq!(resize_due(&mut last, None), None);
		assert_eq!(last, Some((24, 80)));
		// Changed: due, and recorded.
		assert_eq!(resize_due(&mut last, Some((30, 120))), Some((30, 120)));
		assert_eq!(last, Some((30, 120)));
		// The same change again: already reported.
		assert_eq!(resize_due(&mut last, Some((30, 120))), None);
	}
}
