//! Escaping of control characters in text podup prints that it did not write
//! itself: names, paths and values that come from a compose file, the command
//! line or the engine's answers.

use std::borrow::Cow;

/// Whether the log escaping applies to `c`: everything the tables escape except
/// the line feed and tab the message layout uses.
fn unsafe_here(c: char) -> bool {
	podup::ui::is_terminal_unsafe(c) && c != '\n' && c != '\t'
}

/// Escape every control character except line feed and tab, so a name inside a
/// log message cannot start a terminal sequence (ESC, C1 such as U+009B), move
/// the cursor to overwrite the text before it (CR, backspace) or hide text
/// (NUL, BEL). A line feed stays because the message layout uses it; a name
/// that carries one is still bounded by the `podup:` prefix of the next line.
///
/// Bidirectional and zero-width characters are escaped too, since they reorder
/// or hide the text around them.
///
/// Borrows when nothing needs escaping.
pub(super) fn escape_log_controls(s: &str) -> Cow<'_, str> {
	if !s.chars().any(unsafe_here) {
		return Cow::Borrowed(s);
	}
	let mut out = String::with_capacity(s.len() + 8);
	for c in s.chars() {
		if unsafe_here(c) {
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
	if !json.chars().any(escaped_in_json) {
		return Cow::Borrowed(json);
	}
	let mut out = String::with_capacity(json.len() + 8);
	for c in json.chars() {
		if escaped_in_json(c) {
			out.push_str(&format!("\\u{:04x}", c as u32));
		} else {
			out.push(c);
		}
	}
	Cow::Owned(out)
}

/// DEL, the C1 controls and the bidirectional and zero-width characters, none
/// of which `serde_json` escapes.
fn escaped_in_json(c: char) -> bool {
	('\u{7f}'..='\u{9f}').contains(&c) || (podup::ui::is_terminal_unsafe(c) && !c.is_ascii())
}

/// Brackets a character that the YAML emitter would print raw. A control
/// character inside a string makes the emitter double-quote it and write the
/// character as an escape, so this carries the code point through that escape.
const MARK: char = '\u{1}';

/// Make the YAML emitter write the bidirectional and zero-width characters of
/// every string and key as `\uXXXX` escapes in a double-quoted scalar, as it
/// already does for control characters. The emitter would print them as they
/// are; [`unmark_yaml`] turns the marks it leaves back into YAML escapes.
pub(super) fn mark_yaml_unsafe(v: &mut serde_yaml::Value) {
	use serde_yaml::Value;
	match v {
		Value::String(s) if s.chars().any(escaped_in_yaml) => *s = marked(s),
		Value::Sequence(items) => items.iter_mut().for_each(mark_yaml_unsafe),
		Value::Mapping(map) => {
			let entries: Vec<_> = std::mem::take(map).into_iter().collect();
			for (mut key, mut value) in entries {
				mark_yaml_unsafe(&mut key);
				mark_yaml_unsafe(&mut value);
				map.insert(key, value);
			}
		}
		Value::Tagged(tagged) => mark_yaml_unsafe(&mut tagged.value),
		_ => {}
	}
}

fn escaped_in_yaml(c: char) -> bool {
	!c.is_ascii() && podup::ui::is_terminal_unsafe(c) && !('\u{7f}'..='\u{9f}').contains(&c)
}

fn marked(s: &str) -> String {
	let mut out = String::with_capacity(s.len() + 16);
	for c in s.chars() {
		if escaped_in_yaml(c) {
			out.push_str(&format!("{MARK}U{:04X}{MARK}", c as u32));
		} else {
			out.push(c);
		}
	}
	out
}

/// Replace what [`mark_yaml_unsafe`] left in the emitted text (`\x01U202E\x01`)
/// with a YAML four-digit unicode escape. Text that merely looks like a mark ends up
/// as an escape too, which is still safe to print.
pub(super) fn unmark_yaml(yaml: &str) -> String {
	let mut out = String::with_capacity(yaml.len());
	let mut rest = yaml;
	while let Some(at) = rest.find("\\x01U") {
		let hex = &rest[at + 5..];
		match hex.get(..4).zip(hex.get(4..8)) {
			Some((digits, "\\x01")) if digits.chars().all(|c| c.is_ascii_hexdigit()) => {
				out.push_str(&rest[..at]);
				out.push_str("\\u");
				out.push_str(digits);
				rest = &hex[8..];
			}
			_ => {
				out.push_str(&rest[..at + 5]);
				rest = hex;
			}
		}
	}
	out.push_str(rest);
	out
}
