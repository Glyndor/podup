//! Lifecycle progress lines, and the process-wide switch that gates them.
//!
//! One line format for every caller, in the shape docker compose uses for its
//! own progress stream. The switch lives here rather than with the board
//! because it gates this writing, not the board: a command with no board open
//! still emits the same lines through here.

use super::super::identity_style;
use super::super::{progress, AnsiColor, Style};

/// Process-wide switch for user-facing lifecycle progress output. Off by default
/// so the library (consumed by other crates) stays silent unless asked; the CLI
/// turns it on for the human-facing lifecycle commands. Progress is written to
/// stderr (matching docker compose) so stdout stays a clean pipe for scripting
/// and machine/JSON output.
static PROGRESS_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Enable or disable user-facing lifecycle progress output process-wide. The CLI
/// enables it for the lifecycle commands; embedders that want podup silent leave
/// it off (the default).
pub fn set_progress(enabled: bool) {
	PROGRESS_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Whether user-facing lifecycle progress output is currently enabled.
pub fn progress_enabled() -> bool {
	PROGRESS_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Emit a concise per-resource lifecycle progress line to stderr, mirroring
/// docker compose's `Container <name>  <Action>` progress stream (which also
/// goes to stderr). `kind` is the resource noun (`Container`, `Network`,
/// `Volume`, `Image`); `action` is the past-tense verb (`Started`, `Removed`,
/// …), tinted green on a colour sink. A no-op unless [`set_progress`] enabled
/// progress output, so stdout consumers and machine output are never polluted.
pub fn progress_line(kind: &str, name: &str, action: &str) {
	// Test-only recording: lives next to the gate so a test that runs with
	// `PROGRESS_ENABLED=false` (the cargo-test default, and what every library
	// embedder sees) can still observe that the closing verb reached
	// `progress_line`. Compiled out of release builds. The actual rendering
	// gate below is untouched, so production output is identical.
	#[cfg(test)]
	if let Some(kind) = progress::Kind::from_noun(kind) {
		progress::capture::record_finish(kind, name, action);
	}
	if !progress_enabled() {
		return;
	}
	// A board, when one is open, owns the rendering: it needs the event as a
	// state change rather than as a line, and it decides where on screen the
	// row goes. Routing here rather than at each site is what let the board
	// arrive without touching the 21 call sites that report an ending.
	if progress::finish(kind, name, action) {
		return;
	}
	write_progress_line(kind, name, action);
}

/// Write one progress line to stderr, with no board routing.
///
/// Split out of [`progress_line`] because the plain sink has to emit exactly
/// this and cannot go back through the routing that sent it here; that would
/// be a loop. It is also the reason the two sinks cannot drift: there is one
/// line format, used by both.
pub(crate) fn write_progress_line(kind: &str, name: &str, action: &str) {
	use std::io::Write;
	let verb = action_style(action);
	let ident = identity_style(name);
	// The name is the engine's or the compose file's, not ours: a carriage
	// return or a C1 control in it would rewrite the line it sits on.
	let name = crate::error::sanitize_name(name);
	let action = crate::error::sanitize_name(action);
	// anstream::stderr strips the ANSI codes itself when colour is off.
	let _ = writeln!(
		anstream::stderr(),
		" {kind} {}{name}{}  {}{action}{}",
		ident.render(),
		ident.render_reset(),
		verb.render(),
		verb.render_reset()
	);
}

/// The colour band for a lifecycle verb.
///
/// Every verb used to be the same green, so `Volume data Removed`, which
/// destroys data and cannot be undone, was styled exactly like `Started`. The
/// bands say what kind of thing happened: something now exists (green),
/// something stopped but survives (yellow), something is gone (red), nothing
/// changed (dim).
fn action_style(action: &str) -> Style {
	let a = action.to_ascii_lowercase();
	// `unpaused` is checked before `paus` on purpose. It reaches the green arm
	// either way (a container resuming is a thing becoming active, which is what
	// green means here) but only by falling through, and adding `unpause` to the
	// yellow arm's prefixes would silently invert it. Naming it pins the intent.
	// `fail` joins the red arm so a row closing with verb "Failed" reads as a
	// failure in colour, not just in word (#1347).
	if a.starts_with("unpaus") {
		Style::new().fg_color(Some(AnsiColor::Green.into()))
	} else if a.starts_with("remov")
		|| a.starts_with("kill")
		|| a.starts_with("delet")
		|| a.starts_with("fail")
	{
		Style::new().fg_color(Some(AnsiColor::Red.into()))
	} else if a.starts_with("stop") || a.starts_with("paus") || a.starts_with("restart") {
		Style::new().fg_color(Some(AnsiColor::Yellow.into()))
	} else if a.starts_with("exist") || a.starts_with("running") || a.starts_with("skip") {
		Style::new().dimmed()
	} else {
		Style::new().fg_color(Some(AnsiColor::Green.into()))
	}
}

/// Emit a plain user-facing progress note to stderr (e.g. a "nothing to do"
/// line). A no-op unless [`set_progress`] enabled progress output.
pub fn progress_note(msg: &str) {
	use std::io::Write;
	if !progress_enabled() {
		return;
	}
	let _ = writeln!(anstream::stderr(), "{msg}");
}

/// Print a result line to stdout, used by `run -d` to echo the started
/// container's name so scripts capturing stdout get the id, matching
/// `docker compose run -d`. A no-op unless [`set_progress`] enabled progress
/// output, so embedders and machine paths stay silent.
///
/// The result line is the one thing scripts capture from stdout, so a failed
/// write is a real error (exit non-zero), not something to swallow, except a
/// broken pipe, which follows the process-wide quiet-exit convention.
pub fn result_line(msg: &str) -> std::io::Result<()> {
	use std::io::Write;
	if !progress_enabled() {
		return Ok(());
	}
	match writeln!(anstream::stdout(), "{msg}") {
		Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e),
		_ => Ok(()),
	}
}

#[cfg(test)]
#[path = "lines_tests.rs"]
mod tests;
