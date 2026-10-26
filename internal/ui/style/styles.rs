//! The styling primitives the surfaces draw from: wrap text in a style, bold,
//! and the bold red the `error:` label wears.
//!
//! Deliberately the lowest layer in this module, below the surfaces that decide
//! what a thing means. Nothing here reads the colour choice: an anstream sink
//! strips what it is given, and a raw sink asks [`stdout_colored`](super::super::stdout_colored)
//! first and passes the answer in as `enabled`.

use super::super::AnsiColor;
use super::super::Style;

/// Bold, for table headers and emphasis.
pub fn bold() -> Style {
	Style::new().bold()
}

/// Bold red, for the `error:` label.
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

#[cfg(test)]
#[path = "styles_tests.rs"]
mod tests;
