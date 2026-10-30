use super::sanitize_value;

/// Keys whose value Quadlet/systemd splits on whitespace (a `KEY=VALUE` option
/// list): an unquoted space would split a single value into a truncated value
/// plus bogus extra entries, so such values are quoted when they carry
/// anything the reader would otherwise consume.
///
/// `Secret` belongs to the set because Quadlet reads it the same way: it takes
/// the value apart with an exec-like split, so a secret whose path holds a
/// space would reach `podman` as two `--secret` flags, each one broken.
fn key_is_word_split(key: &str) -> bool {
	matches!(
		key,
		"Environment" | "Label" | "Annotation" | "Sysctl" | "Secret"
	)
}

/// Whether a word-split value carries a character that would end the word or
/// change its content if it were written unquoted.
///
/// The reader splits these values on whitespace and honours quotes and
/// C-style escapes inside them, so a `"`, a `'` or a `\` in the middle of the
/// value is consumed: the quotes disappear, the escape eats the character
/// after it, and a quote that never closes costs the whole entry.
fn value_needs_quoting(value: &str) -> bool {
	value
		.chars()
		.any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '\\'))
}

/// Escape the body of a quoted unit value: the characters the reader would
/// otherwise consume (`\` and `"`), plus the control characters a multi-line
/// value carries, which become the C escapes the reader decodes back into the
/// character they stand for.
///
/// A control character the reader has no escape for is dropped, the same rule
/// [`sanitize_value`] applies: a value never carries a raw newline onto the
/// unit file, where it would split the directive.
fn quote_unit_value(value: &str) -> String {
	let mut out = String::with_capacity(value.len());
	for c in value.chars() {
		match c {
			'\\' => out.push_str("\\\\"),
			'"' => out.push_str("\\\""),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if c.is_control() => {}
			_ => out.push(c),
		}
	}
	out
}

/// Keys whose value is an argument line we render (and quote) ourselves
/// (`PodmanArgs`, `Exec`, `Entrypoint`); systemd must keep splitting these on
/// whitespace, so they are passed through with only control characters stripped.
fn key_is_arg_line(key: &str) -> bool {
	matches!(key, "PodmanArgs" | "Exec" | "Entrypoint")
}

/// Escape a value for a single-line systemd `Key=Value` entry.
///
/// On top of [`sanitize_value`] (control characters stripped so a value can
/// never inject a second directive) this makes the value byte-faithful to the
/// compose source the way docker-compose treats it:
///
/// * systemd specifiers (`%h`, `%U`, …) are passed through literally by doubling
///   `%` to `%%`, instead of being expanded at unit-activation time;
/// * a value ending in a backslash, which would otherwise fold the next
///   physical line (and the directive on it) into this value, is quoted and the
///   backslash escaped, closing the line-continuation hole;
/// * for word-split keys (`Environment`, `Label`, …) a value carrying
///   whitespace, a quote, an apostrophe, a backslash or a control character is
///   double-quoted and escaped, so the reader keeps it as one value with the
///   content the compose file declared instead of splitting it into bogus
///   extra entries, eating the quotes, consuming an escape or dropping the
///   entry on an unclosed quote.
///
/// Argument-line keys keep their own rendering (they encode whitespace splitting
/// deliberately) and only have control characters stripped.
pub(super) fn escape_unit_value(key: &str, value: &str) -> String {
	if key_is_arg_line(key) {
		return sanitize_value(value);
	}
	let needs_quote = (key_is_word_split(key) && value_needs_quoting(value))
		|| value.ends_with('\\')
		|| value.starts_with('"');
	let body = if needs_quote {
		quote_unit_value(value)
	} else {
		sanitize_value(value)
	};
	let body = body.replace('%', "%%");
	if needs_quote {
		format!("\"{}\"", body)
	} else {
		body
	}
}

/// A single `[Section]` accumulating `Key=Value` lines in insertion order.
pub(in super::super) struct Section {
	name: &'static str,
	lines: Vec<String>,
}

impl Section {
	/// Start an empty section with the given `[name]` header.
	pub(in super::super) fn new(name: &'static str) -> Self {
		Section {
			name,
			lines: Vec::new(),
		}
	}

	/// Append a `Key=Value` line, escaping the value for systemd unit syntax
	/// (control characters stripped, specifiers passed through literally,
	/// whitespace/line-continuation hazards quoted) so a hostile or merely
	/// awkward compose field cannot inject extra directives, split a value, or
	/// swallow the following line. See [`escape_unit_value`].
	pub(in super::super) fn add(&mut self, key: &str, value: String) {
		let mut value = escape_unit_value(key, &value);
		// Quadlet copies these values into `ExecStart=`, where systemd would
		// substitute `$NAME`; `$$` keeps the dollar literal. `[Unit]` and
		// `[Service]` values are not command lines.
		if !matches!(self.name, "Unit" | "Service") {
			value = value.replace('$', "$$");
		}
		self.lines.push(format!("{key}={value}"));
	}

	/// True when no lines have been added; lets the caller skip emitting an empty
	/// section.
	pub(in super::super) fn is_empty(&self) -> bool {
		self.lines.is_empty()
	}

	/// Render the section as `[name]` followed by its lines, each newline-terminated.
	pub(in super::super) fn render(&self) -> String {
		let mut s = format!("[{}]\n", self.name);
		for line in &self.lines {
			s.push_str(line);
			s.push('\n');
		}
		s
	}
}
