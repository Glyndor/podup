//! Escaping of control characters in text podup prints that it did not write
//! itself: names, paths and values that come from a compose file, the command
//! line or the engine's answers.

use std::borrow::Cow;

/// Escape every control character except line feed and tab, so a name inside a
/// log message cannot start a terminal sequence (ESC, C1 such as U+009B), move
/// the cursor to overwrite the text before it (CR, backspace) or hide text
/// (NUL, BEL). A line feed stays because the message layout uses it; a name
/// that carries one is still bounded by the `podup:` prefix of the next line.
///
/// Borrows when nothing needs escaping.
pub(super) fn escape_log_controls(s: &str) -> Cow<'_, str> {
	if !s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
		return Cow::Borrowed(s);
	}
	let mut out = String::with_capacity(s.len() + 8);
	for c in s.chars() {
		if c.is_control() && c != '\n' && c != '\t' {
			out.extend(c.escape_default());
		} else {
			out.push(c);
		}
	}
	Cow::Owned(out)
}

/// Rewrite DEL and the C1 controls (U+007F to U+009F) in rendered JSON as
/// `\u00XX` escapes. `serde_json` escapes only the characters below U+0020, so
/// these reach the terminal raw, where U+009B alone opens a control sequence.
/// Outside a string these characters cannot occur in JSON, so the rewrite keeps
/// the document valid and decoding it returns the same value.
pub(super) fn escape_json_controls(json: &str) -> Cow<'_, str> {
	if !json.chars().any(|c| ('\u{7f}'..='\u{9f}').contains(&c)) {
		return Cow::Borrowed(json);
	}
	let mut out = String::with_capacity(json.len() + 8);
	for c in json.chars() {
		if ('\u{7f}'..='\u{9f}').contains(&c) {
			out.push_str(&format!("\\u{:04x}", c as u32));
		} else {
			out.push(c);
		}
	}
	Cow::Owned(out)
}
