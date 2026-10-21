//! Bold column headers, the one row every list command prints above its data.
//!
//! Rendered through an anstream sink so the styling is stripped automatically
//! when colour is off, which is what lets [`Table::print_to`](super::Table::print_to)
//! write plain body columns beside a header it did not have to strip itself.

use super::bold;

/// Print `cols` as a bold table header through an anstream sink, so the styling
/// is stripped automatically when colour is off. The single place every list
/// command renders its header, keeping them consistent.
pub fn print_bold_header(cols: &str) {
	let stdout = std::io::stdout();
	let mut out = stdout.lock();
	let _ = write_bold_header(&mut out, cols);
}

/// [`print_bold_header`] writing to `w`, factored out so the renderer can be
/// driven against an in-memory buffer. The bold styling is rendered as the
/// raw ANSI codes regardless of the global colour choice: tests read the
/// bytes verbatim, and `Table::print_to` (the only caller here) writes plain
/// columns alongside the header.
pub fn write_bold_header<W: std::io::Write>(w: &mut W, cols: &str) -> std::io::Result<()> {
	let s = bold();
	writeln!(w, "{}{cols}{}", s.render(), s.render_reset())
}
