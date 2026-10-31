//! Shared scan for terminal control sequences in command output.

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
