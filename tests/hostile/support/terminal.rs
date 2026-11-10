//! Shared scan for terminal control sequences in command output.

// Each test file is its own crate and uses part of this.
#![allow(dead_code)]

/// Everything that starts a terminal sequence, as one string.
pub const PAYLOAD: &str = "\u{1b}]0;PWNED\u{7}\u{1b}[2JRED\u{8}X";

/// A compose or JSON scalar holding `s`, double-quoted.
pub fn quoted(s: &str) -> String {
	let mut out = String::from("\"");
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// `text` with podup's own styling removed: `ESC [` digits and semicolons `m`.
/// The payload has no such sequence, so what this removes is never the payload.
fn without_own_styling(text: &str) -> String {
	let mut out = String::new();
	let mut rest = text;
	while let Some(at) = rest.find("\u{1b}[") {
		out.push_str(&rest[..at]);
		let after = &rest[at + 2..];
		let len = after
			.find(|c: char| !(c.is_ascii_digit() || c == ';'))
			.unwrap_or(after.len());
		if after[len..].starts_with('m') {
			rest = &after[len + 1..];
		} else {
			out.push_str("\u{1b}[");
			rest = after;
		}
	}
	out.push_str(rest);
	out
}

/// The output, quoted, when it holds a byte that starts or edits a terminal
/// sequence: ESC, BEL or backspace.
pub fn raw_control(text: &str) -> Option<String> {
	let plain = without_own_styling(text);
	plain
		.chars()
		.any(|c| matches!(c, '\u{1b}' | '\u{7}' | '\u{8}'))
		.then(|| format!("{plain:?}"))
}

/// Text that reorders or hides what the operator reads: bidirectional marks,
/// embeddings, overrides and isolates, plus invisible joiners and spaces. One
/// of each family, so a filter that knows only some of them is caught.
pub const BIDI_PAYLOAD: &str =
	"ok\u{202e}evil\u{2066}x\u{2069}\u{200b}\u{200d}\u{2060}\u{feff}\u{61c}\u{200e}\u{202a}end";

/// Characters of [`BIDI_PAYLOAD`] that must never reach the terminal raw.
pub fn is_invisible_reorder(c: char) -> bool {
	matches!(
		c,
		'\u{200b}'..='\u{200f}'
			| '\u{202a}'..='\u{202e}'
			| '\u{2060}'..='\u{2064}'
			| '\u{2066}'..='\u{2069}'
			| '\u{61c}'
			| '\u{feff}'
	)
}

/// The output, quoted, when it holds a bidirectional or zero-width character.
pub fn raw_bidi(text: &str) -> Option<String> {
	text.chars()
		.any(is_invisible_reorder)
		.then(|| format!("{text:?}"))
}
