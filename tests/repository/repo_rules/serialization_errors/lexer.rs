//! A tokeniser for the sweep: enough Rust to tell code from prose.
//!
//! The rule reads the shape of a call, not its text, so the text it reads has
//! to be the text of the program. A comment that names the old form is prose,
//! and a string literal that holds the old form is data; neither is a call
//! anybody can make. Both are removed here.
//!
//! What survives is normalised rather than dropped: a literal that is empty
//! keeps its emptiness, because `unwrap_or("")` is one of the shapes the rule
//! is about, and a literal that is not empty keeps only the fact that it is
//! not. The same goes for the line a token came from, which is what the
//! report points at.

/// One token of Rust source: its normalised text and the line it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
	/// The normalised text: an identifier as written, a literal reduced to
	/// whether it is empty, and punctuation as written.
	pub text: String,
	/// The line the token starts on, counted from one.
	pub line: usize,
}

/// The tokens of `src`, with the comments removed and the literals normalised.
///
/// Not a parser: it does not know what an expression is, only where the
/// words, the literals and the punctuation are. That is enough, because the
/// rule is about a name followed by a call and a chain of calls.
pub fn tokenize(src: &str) -> Vec<Token> {
	let chars: Vec<char> = src.chars().collect();
	let mut tokens: Vec<Token> = Vec::new();
	let mut index = 0;
	let mut line = 1;
	while index < chars.len() {
		let c = chars[index];
		match c {
			'\n' => {
				line += 1;
				index += 1;
			}
			c if c.is_whitespace() => index += 1,
			'/' if chars.get(index + 1) == Some(&'/') => {
				while index < chars.len() && chars[index] != '\n' {
					index += 1;
				}
			}
			'/' if chars.get(index + 1) == Some(&'*') => {
				index = skip_block_comment(&chars, index, &mut line);
			}
			'"' => {
				let (text, next) = read_string(&chars, index);
				tokens.push(Token { text, line });
				index = next;
			}
			'r' if is_raw_string_start(&chars, index) => {
				let (text, next) = read_raw_string(&chars, index);
				tokens.push(Token { text, line });
				index = next;
			}
			'b' if is_raw_string_start(&chars, index) => {
				let (text, next) = read_raw_string(&chars, index);
				tokens.push(Token { text, line });
				index = next;
			}
			'b' if chars.get(index + 1) == Some(&'"') => {
				let (text, next) = read_string(&chars, index + 1);
				tokens.push(Token { text, line });
				index = next;
			}
			'\'' => {
				// A lifetime (`'a`) and a character literal (`'a'`) look alike
				// until the closing quote: only the second one has one.
				match read_char_literal(&chars, index) {
					Some((text, next)) => {
						tokens.push(Token { text, line });
						index = next;
					}
					None => {
						tokens.push(Token {
							text: "'".to_string(),
							line,
						});
						index += 1;
					}
				}
			}
			c if c.is_alphabetic() || c == '_' => {
				let start = index;
				while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_')
				{
					index += 1;
				}
				tokens.push(Token {
					text: chars[start..index].iter().collect(),
					line,
				});
			}
			c if c.is_ascii_digit() => {
				let start = index;
				while index < chars.len()
					&& (chars[index].is_alphanumeric()
						|| chars[index] == '_'
						|| chars[index] == '.')
				{
					index += 1;
				}
				tokens.push(Token {
					text: chars[start..index].iter().collect(),
					line,
				});
			}
			':' if chars.get(index + 1) == Some(&':') => {
				tokens.push(Token {
					text: "::".to_string(),
					line,
				});
				index += 2;
			}
			c => {
				tokens.push(Token {
					text: c.to_string(),
					line,
				});
				index += 1;
			}
		}
	}
	tokens
}

/// Skip a block comment, which nests in Rust, and return the index after it.
fn skip_block_comment(chars: &[char], start: usize, line: &mut usize) -> usize {
	let mut index = start + 2;
	let mut depth = 1;
	while index < chars.len() && depth > 0 {
		if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
			depth += 1;
			index += 2;
		} else if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
			depth -= 1;
			index += 2;
		} else {
			if chars[index] == '\n' {
				*line += 1;
			}
			index += 1;
		}
	}
	index
}

/// Read a `"…"` literal and normalise it: empty stays empty, anything else
/// becomes a marker that is not empty.
fn read_string(chars: &[char], start: usize) -> (String, usize) {
	let mut index = start + 1;
	let mut empty = true;
	while index < chars.len() {
		match chars[index] {
			'\\' => {
				empty = false;
				index += 2;
			}
			'"' => {
				index += 1;
				break;
			}
			'\n' => {
				empty = false;
				index += 1;
			}
			c => {
				if c != ' ' && c != '\t' {
					empty = false;
				}
				index += 1;
			}
		}
	}
	(normalised_literal(empty), index)
}

/// Read a raw string (`r"…"`, `r#"…"#`) or a byte raw string (`br"…"`).
fn read_raw_string(chars: &[char], start: usize) -> (String, usize) {
	let mut cursor = start;
	if chars[cursor] == 'b' {
		cursor += 1;
	}
	cursor += 1; // the `r`
	let mut hashes = 0;
	while chars.get(cursor) == Some(&'#') {
		hashes += 1;
		cursor += 1;
	}
	let mut index = cursor + 1;
	let mut empty = true;
	while index < chars.len() {
		if chars[index] == '"' {
			let mut closing = index + 1;
			while chars.get(closing) == Some(&'#') {
				closing += 1;
			}
			if closing - index - 1 == hashes {
				return (normalised_literal(empty), closing);
			}
		}
		if chars[index] != ' ' && chars[index] != '\t' && chars[index] != '\n' {
			empty = false;
		}
		index += 1;
	}
	(normalised_literal(empty), index)
}

/// Whether the text at `index` opens a raw or byte string.
fn is_raw_string_start(chars: &[char], index: usize) -> bool {
	let mut cursor = index;
	if chars[cursor] == 'b' {
		cursor += 1;
	}
	if chars.get(cursor) != Some(&'r') {
		return false;
	}
	cursor += 1;
	while chars.get(cursor) == Some(&'#') {
		cursor += 1;
	}
	matches!(chars.get(cursor), Some('"'))
}

/// Read a character literal, or nothing when the quote opens a lifetime.
fn read_char_literal(chars: &[char], start: usize) -> Option<(String, usize)> {
	let mut index = start + 1;
	if chars.get(index) == Some(&'\\') {
		index += 2;
	} else {
		index += 1;
	}
	if chars.get(index) == Some(&'\'') {
		return Some(("'c'".to_string(), index + 1));
	}
	None
}

/// The text a literal is read as: empty stays empty, anything else is not.
fn normalised_literal(empty: bool) -> String {
	if empty {
		"\"\"".to_string()
	} else {
		"\"…\"".to_string()
	}
}
