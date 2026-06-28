//! Minimal dotenv parser shared by `env_file` loading and `.env` interpolation.
//!
//! Implements the subset of compose-spec dotenv rules podup relies on:
//! full-line comments, an optional `export` prefix, single- and double-quoted
//! values (including values that span multiple lines), inline comments on
//! unquoted values, and the standard double-quote escape sequences. Callers
//! decide duplicate-key precedence by the order pairs are returned in.

/// Parse dotenv `content` into ordered `(key, value)` pairs (lenient).
///
/// Pairs are returned in file order; a later duplicate key appears after an
/// earlier one, leaving the precedence decision to the caller. This variant is
/// used for the optional default `.env`: it never fails, so an unterminated
/// quoted value degrades to consuming the rest of the file (historical
/// behaviour) rather than erroring.
pub fn parse(content: &str) -> Vec<(String, String)> {
	// `strict = false` can never produce an error.
	parse_inner(content, false).unwrap_or_default()
}

/// Like [`parse`] but rejects malformed input.
///
/// Used for explicitly requested env files (`--env-file`, a service `env_file:`)
/// where a typo'd or truncated file must fail loudly rather than silently drop
/// variables — matching docker compose, which hard-errors on an unterminated
/// quoted value.
pub fn parse_strict(content: &str) -> crate::error::Result<Vec<(String, String)>> {
	parse_inner(content, true)
}

fn parse_inner(content: &str, strict: bool) -> crate::error::Result<Vec<(String, String)>> {
	// Strip a leading UTF-8 BOM so the first key is not captured as
	// `\u{feff}KEY` (which would silently lose that variable). Matches
	// docker/godotenv, which drop a leading BOM before parsing.
	let content = content.strip_prefix('\u{feff}').unwrap_or(content);

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
			// A bare key (no `=`) means "pass the value through from the host
			// environment" (compose env_file semantics). If the host doesn't
			// define it, the variable is omitted, not set to an empty string.
			let key = line.trim();
			if !key.is_empty() {
				if let Ok(val) = std::env::var(key) {
					out.push((key.to_string(), val));
				}
			}
			continue;
		};

		let key = line[..eq].trim();
		if key.is_empty() {
			continue;
		}
		let value = parse_value(line[eq + 1..].trim_start(), &mut lines, key, strict)?;
		out.push((key.to_string(), value));
	}

	Ok(out)
}

/// Parse the value portion of an assignment, consuming continuation lines for
/// a quoted value that does not close on the first line. In `strict` mode a
/// quote that never closes is a hard error instead of swallowing the rest of
/// the file (which would silently drop every following key).
fn parse_value(
	rest: &str,
	lines: &mut std::str::Lines,
	key: &str,
	strict: bool,
) -> crate::error::Result<String> {
	match rest.chars().next() {
		Some(quote @ ('"' | '\'')) => {
			let body = &rest[quote.len_utf8()..];
			if let Some(end) = find_closing(body, quote) {
				return Ok(unescape(&body[..end], quote));
			}
			// Unterminated on this line: a multi-line quoted value.
			let mut buf = String::from(body);
			for next in lines.by_ref() {
				buf.push('\n');
				if let Some(end) = find_closing(next, quote) {
					buf.push_str(&next[..end]);
					return Ok(unescape(&buf, quote));
				}
				buf.push_str(next);
			}
			if strict {
				return Err(crate::error::ComposeError::EnvFile(format!(
					"unterminated quoted value for key '{key}'"
				)));
			}
			Ok(unescape(&buf, quote))
		}
		_ => Ok(strip_inline_comment(rest).trim_end().to_string()),
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
	fn double_quoted_expands_all_escape_kinds() {
		// `\r`, `\\`, `\"`, `\'`, and an unknown escape (`\z` → `z`) all resolve.
		assert_eq!(map("FOO=\"a\\rb\"\n")["FOO"], "a\rb");
		assert_eq!(map("FOO=\"x\\\\y\"\n")["FOO"], "x\\y");
		assert_eq!(map("FOO=\"q\\\"q\"\n")["FOO"], "q\"q");
		assert_eq!(map("FOO=\"p\\'p\"\n")["FOO"], "p'p");
		assert_eq!(map("FOO=\"a\\zb\"\n")["FOO"], "azb");
	}

	#[test]
	fn export_prefix_bare_key_passes_through_host() {
		// `export NAME` with no `=` passes NAME through from the host environment.
		std::env::set_var("PODUP_DOTENV_EXPORT_BARE", "fromhost");
		let m = map("export PODUP_DOTENV_EXPORT_BARE\n");
		assert_eq!(
			m.get("PODUP_DOTENV_EXPORT_BARE").map(String::as_str),
			Some("fromhost")
		);
		std::env::remove_var("PODUP_DOTENV_EXPORT_BARE");
	}

	#[test]
	fn double_quoted_value_spanning_multiple_lines() {
		// A double-quoted value left open on its line continues until the closing
		// quote on a later line; the newline is preserved.
		let m = map("FOO=\"line one\nline two\"\nBAR=after\n");
		assert_eq!(m["FOO"], "line one\nline two");
		assert_eq!(m["BAR"], "after");
	}

	#[test]
	fn export_prefix_stripped() {
		assert_eq!(map("export FOO=bar\n")["FOO"], "bar");
	}

	#[test]
	fn bare_key_passes_through_or_is_omitted() {
		// Present in the host → passed through; absent → omitted (not empty string).
		std::env::set_var("PODUP_DOTENV_BARE_PRESENT", "v");
		std::env::remove_var("PODUP_DOTENV_BARE_ABSENT");
		let m = map("PODUP_DOTENV_BARE_PRESENT\nPODUP_DOTENV_BARE_ABSENT\n");
		assert_eq!(
			m.get("PODUP_DOTENV_BARE_PRESENT").map(String::as_str),
			Some("v")
		);
		assert!(!m.contains_key("PODUP_DOTENV_BARE_ABSENT"));
		std::env::remove_var("PODUP_DOTENV_BARE_PRESENT");
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
	fn strips_leading_utf8_bom() {
		// A file saved as UTF-8-with-BOM must not capture the first key as
		// `\u{feff}FOO`; the BOM is stripped so FOO resolves normally.
		let m = map("\u{feff}FOO=bar\nBAZ=qux\n");
		assert_eq!(m.get("FOO").map(String::as_str), Some("bar"));
		assert_eq!(m.get("BAZ").map(String::as_str), Some("qux"));
		assert!(!m.keys().any(|k| k.starts_with('\u{feff}')));
	}

	#[test]
	fn parse_strict_strips_leading_bom() {
		let pairs = super::parse_strict("\u{feff}FOO=bar\n").unwrap();
		assert_eq!(pairs, vec![("FOO".to_string(), "bar".to_string())]);
	}

	#[test]
	fn parse_strict_errors_on_unterminated_quote() {
		// An unterminated quote would otherwise absorb every following key into
		// one value, silently dropping them. Strict parsing rejects it.
		let err = super::parse_strict("A=\"oops\nB=keep\n").unwrap_err();
		let msg = err.to_string();
		assert!(msg.contains("unterminated"), "got: {msg}");
		assert!(msg.contains('A'), "should name the offending key: {msg}");
	}

	#[test]
	fn parse_strict_ok_on_terminated_multiline() {
		// A properly closed multi-line value still parses in strict mode and the
		// following key survives.
		let pairs = super::parse_strict("FOO=\"line one\nline two\"\nBAR=after\n").unwrap();
		let m: std::collections::HashMap<_, _> = pairs.into_iter().collect();
		assert_eq!(m["FOO"], "line one\nline two");
		assert_eq!(m["BAR"], "after");
	}

	#[test]
	fn lenient_parse_does_not_error_on_unterminated_quote() {
		// The lenient `.env` path never errors: it degrades to consuming the rest
		// of the file (historical behaviour) rather than failing.
		let pairs = parse("A=\"oops\nB=keep\n");
		assert_eq!(pairs.len(), 1);
		assert_eq!(pairs[0].0, "A");
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
