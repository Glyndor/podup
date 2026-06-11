//! Minimal dotenv parser shared by `env_file` loading and `.env` interpolation.
//!
//! Implements the subset of compose-spec dotenv rules podup relies on:
//! full-line comments, an optional `export` prefix, single- and double-quoted
//! values (including values that span multiple lines), inline comments on
//! unquoted values, and the standard double-quote escape sequences. Callers
//! decide duplicate-key precedence by the order pairs are returned in.

/// Parse dotenv `content` into ordered `(key, value)` pairs.
///
/// Pairs are returned in file order; a later duplicate key appears after an
/// earlier one, leaving the precedence decision to the caller.
pub fn parse(content: &str) -> Vec<(String, String)> {
	let mut out = Vec::new();
	let mut lines = content.lines();

	while let Some(raw) = lines.next() {
		let line = raw.trim_start();
		if line.is_empty() || line.starts_with('#') {
			continue;
		}
		let line = line
			.strip_prefix("export ")
			.map(str::trim_start)
			.unwrap_or(line);

		let Some(eq) = line.find('=') else {
			let key = line.trim();
			if !key.is_empty() {
				out.push((key.to_string(), String::new()));
			}
			continue;
		};

		let key = line[..eq].trim();
		if key.is_empty() {
			continue;
		}
		let value = parse_value(line[eq + 1..].trim_start(), &mut lines);
		out.push((key.to_string(), value));
	}

	out
}

/// Parse the value portion of an assignment, consuming continuation lines for
/// a quoted value that does not close on the first line.
fn parse_value(rest: &str, lines: &mut std::str::Lines) -> String {
	match rest.chars().next() {
		Some(quote @ ('"' | '\'')) => {
			let body = &rest[quote.len_utf8()..];
			if let Some(end) = find_closing(body, quote) {
				return unescape(&body[..end], quote);
			}
			// Unterminated on this line: a multi-line quoted value.
			let mut buf = String::from(body);
			for next in lines.by_ref() {
				buf.push('\n');
				if let Some(end) = find_closing(next, quote) {
					buf.push_str(&next[..end]);
					return unescape(&buf, quote);
				}
				buf.push_str(next);
			}
			unescape(&buf, quote)
		}
		_ => strip_inline_comment(rest).trim_end().to_string(),
	}
}

/// Byte index of the unescaped closing `quote` in `s`, if present.
///
/// Backslash escapes are honoured for double quotes only; single-quoted
/// strings are literal and close at the first quote.
fn find_closing(s: &str, quote: char) -> Option<usize> {
	let bytes = s.as_bytes();
	let q = quote as u8;
	let mut i = 0;
	while i < bytes.len() {
		let c = bytes[i];
		if quote == '"' && c == b'\\' {
			i += 2;
			continue;
		}
		if c == q {
			return Some(i);
		}
		i += 1;
	}
	None
}

/// Expand escape sequences for double-quoted values; single-quoted values are
/// returned verbatim.
fn unescape(s: &str, quote: char) -> String {
	if quote == '\'' {
		return s.to_string();
	}
	let mut out = String::with_capacity(s.len());
	let mut chars = s.chars();
	while let Some(c) = chars.next() {
		if c != '\\' {
			out.push(c);
			continue;
		}
		match chars.next() {
			Some('n') => out.push('\n'),
			Some('r') => out.push('\r'),
			Some('t') => out.push('\t'),
			Some('\\') => out.push('\\'),
			Some('"') => out.push('"'),
			Some('\'') => out.push('\''),
			Some(other) => out.push(other),
			None => out.push('\\'),
		}
	}
	out
}

/// Trim an inline comment from an unquoted value: everything from the first
/// `#` that is preceded by whitespace. A `#` with no preceding whitespace
/// (e.g. directly after `=`) is part of the value.
fn strip_inline_comment(s: &str) -> &str {
	let bytes = s.as_bytes();
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] == b'#' && i > 0 && bytes[i - 1].is_ascii_whitespace() {
			return &s[..i];
		}
		i += 1;
	}
	s
}

#[cfg(test)]
mod tests {
	use super::parse;

	fn map(content: &str) -> std::collections::HashMap<String, String> {
		parse(content).into_iter().collect()
	}

	#[test]
	fn plain_key_value() {
		let m = map("FOO=bar\nBAZ=qux\n");
		assert_eq!(m["FOO"], "bar");
		assert_eq!(m["BAZ"], "qux");
	}

	#[test]
	fn skips_blank_and_comment_lines() {
		let m = map("# header\n\nFOO=bar\n   # indented comment\n");
		assert_eq!(m.len(), 1);
		assert_eq!(m["FOO"], "bar");
	}

	#[test]
	fn strips_double_quotes() {
		assert_eq!(map("FOO=\"bar\"\n")["FOO"], "bar");
	}

	#[test]
	fn strips_single_quotes() {
		assert_eq!(map("FOO='bar'\n")["FOO"], "bar");
	}

	#[test]
	fn double_quoted_keeps_inner_hash() {
		assert_eq!(map("FOO=\"a # b\"\n")["FOO"], "a # b");
	}

	#[test]
	fn single_quoted_is_literal() {
		assert_eq!(map("FOO='a\\nb'\n")["FOO"], "a\\nb");
	}

	#[test]
	fn double_quoted_expands_escapes() {
		assert_eq!(map("FOO=\"a\\nb\\tc\"\n")["FOO"], "a\nb\tc");
	}

	#[test]
	fn unquoted_strips_inline_comment() {
		assert_eq!(map("FOO=bar # trailing\n")["FOO"], "bar");
	}

	#[test]
	fn unquoted_keeps_leading_hash_without_space() {
		assert_eq!(map("FOO=#notacomment\n")["FOO"], "#notacomment");
	}

	#[test]
	fn unquoted_keeps_internal_spaces() {
		assert_eq!(map("FOO=a b c\n")["FOO"], "a b c");
	}

	#[test]
	fn export_prefix_stripped() {
		assert_eq!(map("export FOO=bar\n")["FOO"], "bar");
	}

	#[test]
	fn bare_key_has_empty_value() {
		assert_eq!(map("BARE\n")["BARE"], "");
	}

	#[test]
	fn multiline_double_quoted_value() {
		let m = map("FOO=\"line1\nline2\"\nBAR=baz\n");
		assert_eq!(m["FOO"], "line1\nline2");
		assert_eq!(m["BAR"], "baz");
	}

	#[test]
	fn multiline_single_quoted_value() {
		let m = map("FOO='line1\nline2'\n");
		assert_eq!(m["FOO"], "line1\nline2");
	}

	#[test]
	fn later_duplicate_returned_after_earlier() {
		let pairs = parse("FOO=first\nFOO=second\n");
		assert_eq!(
			pairs,
			vec![
				("FOO".to_string(), "first".to_string()),
				("FOO".to_string(), "second".to_string()),
			]
		);
	}
}
