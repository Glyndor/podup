//! Terminal styling — the single place that decides whether to colour output and
//! holds the palette, so colour stays consistent and always honours `--ansi`,
//! `NO_COLOR`, and whether the target stream is a TTY.
//!
//! Two kinds of sink exist in podup:
//! - Sinks written through [`anstream`] (e.g. [`anstream::stdout`]) strip the
//!   ANSI codes themselves when colour is off, so callers may always emit codes.
//! - Raw sinks (the tracing writer, the log-prefixer's borrowed stdout) do not
//!   strip, so those callers gate on `stdout_colored`/`stderr_colored`.
//!
//! `set_color_choice` also constructs the auto streams once so anstream enables
//! virtual-terminal processing on Windows for the rest of the process.

use std::io::IsTerminal;

use anstyle::{AnsiColor, Style};

pub use anstream::ColorChoice;

mod table;
pub use table::{fit_cell, Table};

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
	use std::io::Write;
	if !progress_enabled() {
		return;
	}
	let style = Style::new().fg_color(Some(AnsiColor::Green.into()));
	// anstream::stderr strips the ANSI codes itself when colour is off.
	let _ = writeln!(
		anstream::stderr(),
		" {kind} {name}  {}{action}{}",
		style.render(),
		style.render_reset()
	);
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

/// Print a result line to stdout — used by `run -d` to echo the started
/// container's name so scripts capturing stdout get the id, matching
/// `docker compose run -d`. A no-op unless [`set_progress`] enabled progress
/// output, so embedders and machine paths stay silent.
///
/// The result line is the one thing scripts capture from stdout, so a failed
/// write is a real error (exit non-zero), not something to swallow — except a
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

/// Bold — table headers and emphasis.
pub fn bold() -> Style {
	Style::new().bold()
}

/// Print `cols` as a bold table header through an anstream sink, so the styling
/// is stripped automatically when colour is off. The single place every list
/// command renders its header, keeping them consistent.
pub fn print_bold_header(cols: &str) {
	use std::io::Write;
	let s = bold();
	let _ = writeln!(
		anstream::stdout(),
		"{}{cols}{}",
		s.render(),
		s.render_reset()
	);
}

/// Bold red — the `error:` label.
pub fn error_style() -> Style {
	Style::new().bold().fg_color(Some(AnsiColor::Red.into()))
}

/// Wrap `text` in `style` when `enabled`, else return it unchanged. For raw sinks
/// that do not strip; anstream sinks should pass the codes through directly.
pub fn paint(style: Style, text: &str, enabled: bool) -> String {
	if enabled {
		format!("{}{text}{}", style.render(), style.render_reset())
	} else {
		text.to_string()
	}
}

/// Identity colours cycled through for per-service log prefixes. Deliberately
/// excludes red/green/yellow, which are reserved for status/severity meaning, so
/// a service prefix is never mistaken for an error/ok/warning signal.
const SERVICE_PALETTE: [AnsiColor; 6] = [
	AnsiColor::Cyan,
	AnsiColor::Magenta,
	AnsiColor::Blue,
	AnsiColor::BrightCyan,
	AnsiColor::BrightMagenta,
	AnsiColor::BrightBlue,
];

/// Stable palette index for a service name — the same name always maps to the
/// same colour (FNV-1a over the bytes, modulo the palette size).
fn palette_index(name: &str) -> usize {
	let mut h: u64 = 0xcbf2_9ce4_8422_2325;
	for b in name.bytes() {
		h ^= u64::from(b);
		h = h.wrapping_mul(0x0100_0000_01b3);
	}
	(h % SERVICE_PALETTE.len() as u64) as usize
}

/// The stable colour for a service's aggregated-log prefix.
pub fn service_style(name: &str) -> Style {
	Style::new().fg_color(Some(SERVICE_PALETTE[palette_index(name)].into()))
}

/// The semantic colour for a container status word, or `None` for an unknown one
/// (left uncoloured). Green = up/healthy, red = exited/dead/unhealthy, yellow =
/// paused/(re)starting, dim = created. Matches on substrings so it handles both
/// the bare state (`running`) and Podman's verbose `Status` (`Up 2 minutes`,
/// `Exited (1)`).
fn status_style(status: &str) -> Option<Style> {
	let s = status.to_ascii_lowercase();
	let colour = if s.contains("unhealthy") || s.contains("exit") || s.contains("dead") {
		AnsiColor::Red
	} else if s.contains("running") || s.contains("healthy") || s.starts_with("up") {
		AnsiColor::Green
	} else if s.contains("paus") || s.contains("restart") || s.contains("starting") {
		AnsiColor::Yellow
	} else if s.contains("created") {
		return Some(Style::new().dimmed());
	} else {
		return None;
	};
	Some(Style::new().fg_color(Some(colour.into())))
}

/// Render a container `status` left-padded to `width`, colourised by its meaning
/// when stdout is a colour sink. The padding is applied first so the colour codes
/// (zero display width) never disturb column alignment.
pub fn status_cell(status: &str, width: usize) -> String {
	let padded = format!("{status:<width$}");
	match status_style(status) {
		Some(style) => paint(style, &padded, stdout_colored()),
		None => padded,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn service_colour_is_stable_per_name() {
		// Same name → same index every call; different names spread across the
		// palette (not all collapsed to one colour).
		assert_eq!(palette_index("web"), palette_index("web"));
		let distinct: std::collections::HashSet<usize> =
			["web", "db", "cache", "worker", "proxy", "queue"]
				.iter()
				.map(|n| palette_index(n))
				.collect();
		assert!(distinct.len() > 1, "palette should spread service names");
		assert!(palette_index("web") < SERVICE_PALETTE.len());
	}

	#[test]
	fn paint_gates_on_enabled() {
		let plain = paint(bold(), "hi", false);
		assert_eq!(plain, "hi");
		let coloured = paint(bold(), "hi", true);
		assert!(coloured.contains("hi"));
		assert!(coloured.len() > "hi".len(), "enabled paint adds ANSI codes");
		assert!(coloured.starts_with('\u{1b}'), "starts with an ESC");
	}

	#[test]
	fn colour_choice_resolution() {
		// Pure resolution — never touches the process-global choice, so it can't
		// race the production code (LinePrefixer/status_cell) that reads it.
		temp_env::with_var_unset("NO_COLOR", || {
			assert!(!colored_with(ColorChoice::Never, true));
			assert!(colored_with(ColorChoice::Always, false));
			assert!(colored_with(ColorChoice::Auto, true));
			assert!(!colored_with(ColorChoice::Auto, false));
		});
		// NO_COLOR forces plain in Auto, regardless of the TTY.
		temp_env::with_var("NO_COLOR", Some("1"), || {
			assert!(!colored_with(ColorChoice::Auto, true));
			// ...but an explicit `always` still overrides NO_COLOR.
			assert!(colored_with(ColorChoice::Always, true));
		});
	}

	#[test]
	fn status_style_is_semantic() {
		assert_ne!(status_style("running"), status_style("exited (1)"));
		assert_ne!(status_style("unhealthy"), status_style("healthy"));
		assert!(status_style("Up 2 minutes").is_some());
		assert!(status_style("paused").is_some());
		assert!(status_style("created").is_some());
		assert!(status_style("weird-state").is_none());
	}

	#[test]
	fn progress_toggle_is_observable() {
		// Off by default-or-restored; toggling flips the observable state. Restore
		// afterwards so the process-global flag does not leak into other tests.
		let prev = progress_enabled();
		set_progress(false);
		assert!(!progress_enabled());
		set_progress(true);
		assert!(progress_enabled());
		set_progress(prev);
	}

	#[test]
	fn status_cell_pads_and_keeps_status() {
		let cell = status_cell("ok", 6);
		assert!(cell.contains("ok"));
		// At least the requested width (colour codes, if any, only add length).
		assert!(cell.len() >= 6);
	}
}
