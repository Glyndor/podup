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
#[cfg(any(test, feature = "test-helpers"))]
pub fn parse(content: &str) -> Vec<(String, String)> {
	// `strict = false` can never produce an error.
	let marked = parse_inner(content, false).unwrap_or_default();
	marked.into_iter().map(|(k, v, _)| (k, v)).collect()
}

/// Like [`parse`], with the third element of [`parse_strict_marked`]: `true`
/// for a value that must not be interpolated. Lenient like [`parse`].
pub fn parse_marked(content: &str) -> Vec<(String, String, bool)> {
	parse_inner(content, false).unwrap_or_default()
}

/// Like [`parse`] but rejects malformed input.
///
/// Used for explicitly requested env files (`--env-file`, a service `env_file:`)
/// where a typo'd or truncated file must fail loudly rather than silently drop
/// variables, matching docker compose, which hard-errors on an unterminated
/// quoted value.
#[cfg(any(test, feature = "test-helpers"))]
pub fn parse_strict(content: &str) -> crate::error::Result<Vec<(String, String)>> {
	let marked = parse_inner(content, true)?;
	Ok(marked.into_iter().map(|(k, v, _)| (k, v)).collect())
}

/// Like [`parse_strict`], with a third element per pair: `true` when the value
/// is literal and must not be interpolated (it was single-quoted, or it is a
/// bare key passed through from the host). Unquoted and double-quoted values
/// are `false`, which is where the Compose Specification applies interpolation.
pub fn parse_strict_marked(content: &str) -> crate::error::Result<Vec<(String, String, bool)>> {
	parse_inner(content, true)
}

/// Parse `content` in the `raw` format of an `env_file:` entry: one `KEY=value`
/// per line with the value exactly as written, quotes, `$` signs, spaces and a
/// trailing `# comment` included, and no interpolation (the Compose
/// Specification: "without any attempt from Compose to parse the value").
/// Blank lines and lines that start with `#` are skipped; a line without `=` is a
/// bare key that passes through from the host if it is set there, as in dotenv.
/// A key that contains whitespace (an `export KEY=value` line, say) is an error,
/// which is what Docker Compose does.
pub fn parse_raw(content: &str) -> crate::error::Result<Vec<(String, String)>> {
	let content = content.strip_prefix('\u{feff}').unwrap_or(content);
	let mut out = Vec::new();
	for line in content.lines() {
		if line.trim().is_empty() || line.trim_start().starts_with('#') {
			continue;
		}
		let Some((key, value)) = line.split_once('=') else {
			let key = line.trim();
			if let Ok(val) = std::env::var(key) {
				out.push((key.to_string(), val));
			}
			continue;
		};
		// Leading blanks are dropped; a key with none left is an error, and so is
		// one with whitespace anywhere after that (`KEY = v`), as in Docker Compose.
		let key = key.trim_start();
		if key.is_empty() {
			return Err(crate::error::ComposeError::EnvFile(format!(
				"no variable name on line '{}'",
				crate::error::sanitize_name(line)
			)));
		}
		if key.contains(char::is_whitespace) {
			return Err(crate::error::ComposeError::EnvFile(format!(
				"variable '{}' contains whitespace",
				crate::error::sanitize_name(key)
			)));
		}
		out.push((key.to_string(), value.to_string()));
	}
	Ok(out)
}

fn parse_inner(content: &str, strict: bool) -> crate::error::Result<Vec<(String, String, bool)>> {
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
					out.push((key.to_string(), val, true));
				}
			}
			continue;
		};

		let key = line[..eq].trim();
		if key.is_empty() {
			continue;
		}
		let rest = line[eq + 1..].trim_start();
		let literal = rest.starts_with('\'');
		let value = parse_value(rest, &mut lines, key, strict)?;
		out.push((key.to_string(), value, literal));
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

/// Expand escape sequences for double-quoted values the way Docker Compose's
/// dotenv reader does (measured against `docker compose config` 5.7.0);
/// single-quoted values are returned verbatim.
///
/// `\$` is a literal dollar. The value is interpolated afterwards, so it is
/// written as `$$`, which interpolation turns back into one `$`; a bare `$`
/// would expand the variable that follows, which is what `\$` asks not to do.
/// An escape Docker does not know keeps its backslash.
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
			// Docker keeps the backslash and drops this one zero (`\077` is `\77`).
			Some('0') => out.push('\\'),
			Some('\\') => out.push('\\'),
			Some('"') => out.push('"'),
			Some('$') => out.push_str("$$"),
			Some(other) => {
				out.push('\\');
				out.push(other);
			}
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
#[path = "dotenv_tests.rs"]
mod tests;
