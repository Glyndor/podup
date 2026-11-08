//! `command:` / `entrypoint:` / `healthcheck.test`: a string or an exec list, and
//! the word splitter that turns the string of `command:` and `entrypoint:` into
//! the argument vector Docker Compose builds from it.

use serde::{Deserialize, Serialize};

/// Container entrypoint / command: either a string or exec list.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Command {
	/// String form. `command:` and `entrypoint:` never keep it: they are split into
	/// [`Command::Exec`] words at load (see [`split_words`]). A healthcheck `test:`
	/// and a lifecycle hook keep it, and run it through `sh -c`.
	Shell(String),
	/// Exec form: an explicit argument vector run without a shell.
	Exec(Vec<String>),
}

impl Command {
	/// Returns the command as an exec argument vector, wrapping a shell string in `sh -c`.
	pub fn to_exec(&self) -> Vec<String> {
		match self {
			Command::Shell(s) => vec!["sh".into(), "-c".into(), s.clone()],
			Command::Exec(v) => v.clone(),
		}
	}

	/// The vector sent as a container's `command:` or `entrypoint:`.
	///
	/// As [`Command::to_exec`], except that a blank string is the empty vector,
	/// the same as `[]`: the Compose Specification says `''` and `[]` both mean
	/// "no value" here (`entrypoint:` "overridden to be empty"), and Docker
	/// Compose splits `""` into no words. Wrapping it in `sh -c ""` ran a shell
	/// that exits at once instead of the image's own command.
	pub fn to_container_override(&self) -> Vec<String> {
		match self {
			Command::Shell(s) if s.trim().is_empty() => Vec::new(),
			other => other.to_exec(),
		}
	}
}

/// Split a `command:` / `entrypoint:` string into words exactly as Docker Compose
/// (v5.7.0, `docker compose config`) does:
///
/// - space, tab, CR and LF separate words; no other character does (a no-break
///   space or a vertical tab stays inside its word);
/// - a single quote keeps everything up to the next single quote;
/// - a double quote keeps everything up to the next double quote;
/// - a backtick, or a `$(` (what `$$(` in the file interpolates to), opens a group
///   that is kept verbatim, delimiters and backslashes included, up to the next
///   backtick or `)` that a backslash does not protect; inside double quotes too;
/// - a backslash outside single quotes and groups drops itself and keeps the next
///   character, except `\n` and `\t`, which are a newline and a tab;
/// - an unquoted `;`, `&`, `|` or `<` ends the parse: the word before it is kept
///   and the rest of the string is dropped without an error; so does a `>`, which
///   also drops a word of bare digits right before it (`2>`, `10>>`), the file
///   descriptor of a redirection;
/// - an unquoted `(` or `)`, an unterminated quote or group and a trailing
///   backslash are errors.
pub fn split_words(text: &str) -> Result<Vec<String>, &'static str> {
	let mut words = Vec::new();
	let mut word = String::new();
	let mut in_word = false;
	// Whether the word so far is bare characters: no quote, no escape, no group.
	let mut bare = true;
	let mut chars = text.chars().peekable();
	while let Some(c) = chars.next() {
		match c {
			' ' | '\t' | '\r' | '\n' => {
				if in_word {
					words.push(std::mem::take(&mut word));
					in_word = false;
					bare = true;
				}
			}
			'>' => {
				if in_word && bare && word.chars().all(|d| d.is_ascii_digit()) {
					in_word = false;
				}
				break;
			}
			';' | '&' | '|' | '<' => break,
			'(' | ')' => return Err("unquoted parenthesis"),
			_ => {
				in_word = true;
				match c {
					'\'' => {
						bare = false;
						loop {
							match chars.next() {
								Some('\'') => break,
								Some(q) => word.push(q),
								None => return Err("unterminated single quote"),
							}
						}
					}
					'"' => {
						bare = false;
						loop {
							match chars.next() {
								Some('"') => break,
								Some('\\') => word.push(escaped(
									chars.next().ok_or("unterminated double quote")?,
								)),
								Some(q) => {
									if !open_group(q, &mut chars, &mut word)? {
										word.push(q);
									}
								}
								None => return Err("unterminated double quote"),
							}
						}
					}
					'\\' => {
						bare = false;
						word.push(escaped(chars.next().ok_or("trailing backslash")?));
					}
					c => {
						if open_group(c, &mut chars, &mut word)? {
							bare = false;
						} else {
							word.push(c);
						}
					}
				}
			}
		}
	}
	if in_word {
		words.push(word);
	}
	Ok(words)
}

/// If `c` opens a backtick or `$(` group, copy the whole group into `word` and
/// return `true`; an unterminated group is an error.
fn open_group(
	c: char,
	chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
	word: &mut String,
) -> Result<bool, &'static str> {
	let close = match (c, chars.peek()) {
		('`', _) => '`',
		('$', Some('(')) => {
			word.push('$');
			word.push(chars.next().unwrap_or('('));
			')'
		}
		_ => return Ok(false),
	};
	if close == '`' {
		word.push('`');
	}
	loop {
		match chars.next() {
			Some('\\') => {
				word.push('\\');
				word.push(chars.next().ok_or("unterminated group")?);
			}
			Some(g) => {
				word.push(g);
				if g == close {
					return Ok(true);
				}
			}
			None => return Err("unterminated group"),
		}
	}
}

/// The character a backslash and `c` stand for.
fn escaped(c: char) -> char {
	match c {
		'n' => '\n',
		't' => '\t',
		other => other,
	}
}

/// `deserialize_with` for `command:` and `entrypoint:`: a string is split into an
/// exec vector as the file loads, so `config`, Quadlet and the engine all see the
/// same words, and a string that cannot be split fails the load.
pub(crate) fn deserialize_split<'de, D>(de: D) -> Result<Option<Command>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	match Option::<Command>::deserialize(de)? {
		Some(Command::Shell(s)) => match split_words(&s) {
			Ok(words) => Ok(Some(Command::Exec(words))),
			Err(why) => Err(serde::de::Error::custom(format!(
				"invalid command line string {s:?}: {why}"
			))),
		},
		other => Ok(other),
	}
}

#[cfg(test)]
#[path = "service/command_split_tests.rs"]
mod split_tests;
