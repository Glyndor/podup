use super::sanitize_value;
use crate::compose::types::Command;

/// Render a compose `command:`/`entrypoint:` into a single-line systemd `Exec=`
/// value. systemd splits the value on whitespace honouring double-quoted groups
/// and C-style escapes, so each list element is quoted when it contains
/// whitespace or quoting characters, and control characters are always escaped.
/// Without this a multi-line argument (e.g. an `sh -c` block scalar) would emit
/// raw newlines that spill onto extra physical lines and corrupt the unit file.
pub(in super::super) fn render_command(command: &Command) -> String {
	match command {
		// A shell-form command is already a single command line; keep its word
		// splitting but neutralise control characters so it stays on one line.
		Command::Shell(s) => escape_exec_control(s),
		Command::Exec(parts) => parts
			.iter()
			.map(|p| quote_exec_arg(p))
			.collect::<Vec<_>>()
			.join(" "),
	}
}

/// C-escape the characters that must never appear raw in a systemd command line.
fn escape_exec_control(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for c in s.chars() {
		match c {
			'\\' => out.push_str("\\\\"),
			// systemd expands specifiers in `Exec` lines; `%%` keeps a literal `%`.
			'%' => out.push_str("%%"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			_ => out.push(c),
		}
	}
	out
}

/// Whether the command has a word that is exactly `;`, or a quote that never
/// closes.
///
/// An unterminated quote is refused too: Quadlet versions differ on it (some
/// drop the command, some pass the rest through as separate words), so no reading
/// of it can be relied on.
///
/// Quadlet unquotes `Exec=` words and writes them to `ExecStart=` without
/// requoting a `;`, and it rejects the `\;` spelling, so such a word reaches
/// systemd as a bare `;` that ends the command. There is no way to write it.
///
/// The check reads the exact text [`render_command`] emits, with the word
/// rules Quadlet applies to it, so the shell form and the exec form share one
/// reading and the check cannot disagree with the parser that matters.
pub(crate) fn has_lone_semicolon_word(command: &Command) -> bool {
	// The text that is written is the rendered one with its control characters
	// stripped (`escape_unit_value` does that for `Exec=`), so that is the text to
	// read: a word like `;` plus a control character is not a lone `;` before the
	// strip and becomes one after it.
	let (words, closed) = exec_words(&sanitize_value(&render_command(command)));
	!closed || words.iter().any(|w| w == ";")
}

/// Split an `Exec=` value into words the way Quadlet and systemd do: white
/// space separates words outside quotes, `'` and `"` open a quoted run anywhere
/// in a word, and a backslash starts a C-style escape both inside and outside
/// quotes (`\n`, `\xHH`, `\NNN`, `\uXXXX`, `\s`, or the escaped character
/// itself). The flag is `false` when a quote never closes.
pub(super) fn exec_words(value: &str) -> (Vec<String>, bool) {
	let mut words = Vec::new();
	let mut word = String::new();
	let mut started = false;
	let mut quote: Option<char> = None;
	let mut chars = value.chars().peekable();
	while let Some(c) = chars.next() {
		match c {
			'\\' => {
				started = true;
				word.push(unescape(&mut chars));
			}
			'"' | '\'' if quote.is_none() => {
				quote = Some(c);
				started = true;
			}
			c if Some(c) == quote => quote = None,
			c if c.is_whitespace() && quote.is_none() => {
				if started {
					words.push(std::mem::take(&mut word));
					started = false;
				}
			}
			c => {
				started = true;
				word.push(c);
			}
		}
	}
	if started {
		words.push(word);
	}
	(words, quote.is_none())
}

/// Decode the escape that follows a backslash.
fn unescape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> char {
	let Some(c) = chars.next() else {
		return '\\';
	};
	let mut number = |radix: u32, max: usize, first: Option<char>| {
		let mut digits: String = first.into_iter().collect();
		while digits.len() < max {
			match chars.peek() {
				Some(d) if d.is_digit(radix) => digits.push(chars.next().unwrap_or_default()),
				_ => break,
			}
		}
		u32::from_str_radix(&digits, radix)
			.ok()
			.and_then(char::from_u32)
			.unwrap_or(char::REPLACEMENT_CHARACTER)
	};
	match c {
		'a' => '\u{7}',
		'b' => '\u{8}',
		'f' => '\u{c}',
		'n' => '\n',
		'r' => '\r',
		't' => '\t',
		'v' => '\u{b}',
		's' => ' ',
		'x' => number(16, 2, None),
		'u' => number(16, 4, None),
		'U' => number(16, 8, None),
		'0'..='7' => number(8, 3, Some(c)),
		other => other,
	}
}

/// Quote a single `Exec=` argument: wrap it in double quotes (escaping `"` and
/// control characters) when it is empty or contains whitespace or quoting
/// characters, so systemd keeps it as one argument. Plain arguments pass through
/// unchanged.
fn quote_exec_arg(arg: &str) -> String {
	let needs_quoting = arg.is_empty()
		|| arg
			.chars()
			.any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '%' | ';'));
	if needs_quoting {
		format!("\"{}\"", escape_exec_control(arg).replace('"', "\\\""))
	} else {
		arg.to_string()
	}
}
