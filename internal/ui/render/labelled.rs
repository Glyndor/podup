//! Labelled lines: a dimmed scaffolding label with a value that carries the
//! meaning, and the headed block a container name introduces.
//!
//! Separate from the status vocabulary because the value is not always a state
//! word: `autostart status` answers yes/no questions, sometimes in prose, and
//! each answer has to land next to the question it answers.

use super::super::identity_style;
use super::super::status::status_style;
use super::super::{paint, AnsiColor, Style};

/// Print a `label: value` line where the label is scaffolding and the value
/// carries the meaning.
///
/// `autostart status` is the densest meaning-per-line surface in the CLI, six
/// consecutive yes/no answers, and it was entirely monochrome, so finding the
/// one line that answers "is it running?" meant reading all six. The label is
/// dimmed and the value tinted by its own status meaning, which covers systemd's
/// vocabulary as well as Podman's.
///
/// Values with no status meaning (a path, a file mode) are left alone rather
/// than given an arbitrary colour.
pub fn print_labelled(label: &str, value: &str) {
	print_labelled_with(label, value, None);
}

/// [`print_labelled`] with the value's meaning stated rather than inferred.
///
/// Some values are prose, not a state word: `XDG_RUNTIME_DIR unset (systemctl
/// --user needs a user session)` is the answer to a yes/no question written as a
/// sentence. `Some(true)`/`Some(false)` colours it green/red; `None` falls back
/// to reading the text.
pub fn print_labelled_with(label: &str, value: &str, good: Option<bool>) {
	use std::io::Write;
	let dim = Style::new().dimmed();
	let padded = format!("{label}:");
	let explicit = good.map(|ok| {
		Style::new().fg_color(Some(
			if ok { AnsiColor::Green } else { AnsiColor::Red }.into(),
		))
	});
	let styled_value = match explicit.or_else(|| status_style(value)) {
		Some(style) => paint(style, value, true),
		None => value.to_string(),
	};
	let _ = writeln!(
		anstream::stdout(),
		"{}{padded:<11}{} {styled_value}",
		dim.render(),
		dim.render_reset()
	);
}

/// [`print_labelled`] with the value dimmed, for an answer that is negative but
/// not a failure.
///
/// `autostart status` needed it: with no unit file on disk, systemd's
/// `is-enabled` answers `not-found`, which the status vocabulary reads as an
/// error and paints red, while the `installed: no` line directly above it says
/// the same thing about the same unit and was dim. Two colours for one fact, and
/// the alarming one belonged to the case where nothing is wrong.
///
/// Separate from [`print_labelled_with`] rather than a third state added to its
/// `Option<bool>`: `ui` is public API and podup is 1.0.0, so the existing
/// signature may not move.
pub fn print_labelled_neutral(label: &str, value: &str) {
	use std::io::Write;
	let dim = Style::new().dimmed();
	let padded = format!("{label}:");
	let _ = writeln!(
		anstream::stdout(),
		"{}{padded:<11}{} {}{value}{}",
		dim.render(),
		dim.render_reset(),
		dim.render(),
		dim.render_reset()
	);
}

/// Print a bold header tinted with its identity colour, used where a block is
/// headed by a container name rather than by column titles.
pub fn print_identity_header(name: &str) {
	use std::io::Write;
	let style = identity_style(name).bold();
	let _ = writeln!(
		anstream::stdout(),
		"{}{name}{}",
		style.render(),
		style.render_reset()
	);
}
