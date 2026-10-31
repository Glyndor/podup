//! Bold column headers, the one row every list command prints above its data.
//!
//! The styling follows the same colour choice as the rest of the output: the
//! header is written to a raw sink, so nothing strips the escapes for it, and
//! it has to ask whether stdout is a colour sink itself.

use super::super::bold;

/// Print `cols` as a bold table header through an anstream sink, so the styling
/// is stripped automatically when colour is off. The single place every list
/// command renders its header, keeping them consistent.
pub fn print_bold_header(cols: &str) {
	let stdout = std::io::stdout();
	let mut out = stdout.lock();
	let _ = write_bold_header(&mut out, cols);
}

/// [`print_bold_header`] writing to `w`, factored out so the renderer can be
/// driven against an in-memory buffer.
///
/// The bold styling is written only when stdout is a colour sink. The header
/// went through a raw sink, so the escapes reached the output whatever the
/// colour choice said: `--ansi never`, `NO_COLOR` and a redirected stdout all
/// asked for plain output and got bold escapes, which is what a script
/// reading `ps` or `images` sees.
pub fn write_bold_header<W: std::io::Write>(w: &mut W, cols: &str) -> std::io::Result<()> {
	write_bold_header_with(w, cols, super::super::stdout_colored())
}

/// [`write_bold_header`] with the colour decision passed in, so the rule is
/// testable without the process-global choice.
pub fn write_bold_header_with<W: std::io::Write>(
	w: &mut W,
	cols: &str,
	colour: bool,
) -> std::io::Result<()> {
	if colour {
		let s = bold();
		writeln!(w, "{}{cols}{}", s.render(), s.render_reset())
	} else {
		writeln!(w, "{cols}")
	}
}

#[cfg(test)]
#[path = "header_tests.rs"]
mod tests;
