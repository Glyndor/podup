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

/// Whether a stream that is (or isn't) a TTY should be coloured under the global
/// choice. `Auto` defers to the TTY and `NO_COLOR`.
fn colored(is_terminal: bool) -> bool {
	match ColorChoice::global() {
		ColorChoice::Always | ColorChoice::AlwaysAnsi => true,
		ColorChoice::Never => false,
		ColorChoice::Auto => is_terminal && !no_color(),
	}
}

/// Whether a raw (non-anstream) write to stdout should embed ANSI codes.
pub fn stdout_colored() -> bool {
	colored(std::io::stdout().is_terminal())
}

/// Whether a raw (non-anstream) write to stderr should embed ANSI codes.
pub fn stderr_colored() -> bool {
	colored(std::io::stderr().is_terminal())
}

/// Bold — table headers and emphasis.
pub fn bold() -> Style {
	Style::new().bold()
}

/// Bold red — the `error:` label.
pub fn error_style() -> Style {
	Style::new().bold().fg_color(Some(AnsiColor::Red.into()))
}

/// Bold yellow — the `warning:` label.
pub fn warn_style() -> Style {
	Style::new().bold().fg_color(Some(AnsiColor::Yellow.into()))
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

/// The distinct colours cycled through for per-service log prefixes.
const SERVICE_PALETTE: [AnsiColor; 6] = [
	AnsiColor::Cyan,
	AnsiColor::Green,
	AnsiColor::Yellow,
	AnsiColor::Magenta,
	AnsiColor::Blue,
	AnsiColor::BrightRed,
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
	fn no_color_env_disables_auto() {
		// `colored` honours NO_COLOR in Auto regardless of the TTY argument.
		temp_env::with_var("NO_COLOR", Some("1"), || {
			ColorChoice::Auto.write_global();
			assert!(!colored(true));
		});
		temp_env::with_var_unset("NO_COLOR", || {
			ColorChoice::Never.write_global();
			assert!(!colored(true));
			ColorChoice::Always.write_global();
			assert!(colored(false));
			ColorChoice::Auto.write_global();
			assert!(colored(true));
			assert!(!colored(false));
		});
		// Restore the default for other tests sharing the process-global choice.
		ColorChoice::Auto.write_global();
	}
}
