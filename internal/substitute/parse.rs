//! `${VAR}` / `$VAR` reference parsing and modifier resolution.
//!
//! Implements the compose-spec modifier forms (`:-`, `-`, `:+`, `+`, `:?`, `?`)
//! by scanning the characters inside a `${…}` group and resolving them against
//! the variable map.

use std::collections::HashMap;

use crate::error::{ComposeError, Result};

pub(super) fn is_var_start(c: char) -> bool {
	c.is_alphabetic() || c == '_'
}

fn is_var_char(c: char) -> bool {
	c.is_alphanumeric() || c == '_'
}

pub(super) fn collect_var_name(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
	let mut name = String::new();
	while let Some(&c) = chars.peek() {
		if is_var_char(c) {
			name.push(c);
			chars.next();
		} else {
			break;
		}
	}
	name
}

#[derive(Debug)]
pub(super) enum Modifier {
	None,
	/// `${VAR:-default}` — use default if unset or empty
	DefaultIfUnsetOrEmpty(String),
	/// `${VAR-default}` — use default if unset (empty value is OK)
	DefaultIfUnset(String),
	/// `${VAR:+value}` — use value if set and non-empty
	AltIfSetAndNonEmpty(String),
	/// `${VAR+value}` — use value if set (even if empty)
	AltIfSet(String),
	/// `${VAR:?error}` — error if unset or empty
	ErrorIfUnsetOrEmpty(String),
	/// `${VAR?error}` — error if unset
	ErrorIfUnset(String),
}

/// Parse the content inside `${…}`.  The opening `{` has already been consumed.
pub(super) fn parse_braced_var(
	chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<(String, Modifier)> {
	let mut name = String::new();

	loop {
		match chars.peek() {
			None => {
				return Ok((name, Modifier::None));
			}
			Some('}') => {
				chars.next();
				return Ok((name, Modifier::None));
			}
			Some(':') => {
				chars.next();
				// Peek at what follows `:`.
				let modifier = match chars.peek() {
					Some('-') => {
						chars.next();
						Modifier::DefaultIfUnsetOrEmpty(collect_until_close(chars))
					}
					Some('+') => {
						chars.next();
						Modifier::AltIfSetAndNonEmpty(collect_until_close(chars))
					}
					Some('?') => {
						chars.next();
						Modifier::ErrorIfUnsetOrEmpty(collect_until_close(chars))
					}
					_ => Modifier::DefaultIfUnsetOrEmpty(collect_until_close(chars)),
				};
				return Ok((name, modifier));
			}
			Some('-') => {
				chars.next();
				return Ok((name, Modifier::DefaultIfUnset(collect_until_close(chars))));
			}
			Some('+') => {
				chars.next();
				return Ok((name, Modifier::AltIfSet(collect_until_close(chars))));
			}
			Some('?') => {
				chars.next();
				return Ok((name, Modifier::ErrorIfUnset(collect_until_close(chars))));
			}
			Some(&c) => {
				name.push(c);
				chars.next();
			}
		}
	}
}

/// Collect everything until `}` (exclusive), consuming the `}`.
fn collect_until_close(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
	let mut buf = String::new();
	for c in chars.by_ref() {
		if c == '}' {
			break;
		}
		buf.push(c);
	}
	buf
}

pub(super) fn resolve_modifier(
	var: String,
	modifier: Modifier,
	vars: &HashMap<String, String>,
) -> Result<String> {
	let value = vars.get(&var);

	match modifier {
		Modifier::None => Ok(value.cloned().unwrap_or_default()),

		Modifier::DefaultIfUnsetOrEmpty(default) => match value {
			Some(v) if !v.is_empty() => Ok(v.clone()),
			_ => Ok(default),
		},

		Modifier::DefaultIfUnset(default) => match value {
			Some(v) => Ok(v.clone()),
			None => Ok(default),
		},

		Modifier::AltIfSetAndNonEmpty(alt) => match value {
			Some(v) if !v.is_empty() => Ok(alt),
			_ => Ok(String::new()),
		},

		Modifier::AltIfSet(alt) => match value {
			Some(_) => Ok(alt),
			None => Ok(String::new()),
		},

		Modifier::ErrorIfUnsetOrEmpty(msg) => match value {
			Some(v) if !v.is_empty() => Ok(v.clone()),
			_ => Err(ComposeError::RequiredVarNotSet { var, msg }),
		},

		Modifier::ErrorIfUnset(msg) => match value {
			Some(v) => Ok(v.clone()),
			None => Err(ComposeError::RequiredVarNotSet { var, msg }),
		},
	}
}
