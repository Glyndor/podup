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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	fn peekable(s: &str) -> std::iter::Peekable<std::str::Chars<'_>> {
		s.chars().peekable()
	}

	fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
		pairs
			.iter()
			.map(|(k, v)| ((*k).to_string(), (*v).to_string()))
			.collect()
	}

	// --- char predicates ---

	#[test]
	fn var_start_and_char_classes() {
		assert!(is_var_start('_'));
		assert!(is_var_start('a'));
		assert!(!is_var_start('1'));
		assert!(!is_var_start('-'));
		assert!(is_var_char('9'));
		assert!(!is_var_char('-'));
	}

	// --- collect_var_name ---

	#[test]
	fn collect_var_name_stops_at_non_var_char() {
		let mut it = peekable("NAME-rest");
		assert_eq!(collect_var_name(&mut it), "NAME");
		// The `-` and everything after it is left unconsumed.
		assert_eq!(it.collect::<String>(), "-rest");
	}

	// --- parse_braced_var: bare + every modifier form ---

	#[test]
	fn parse_bare_var_consumes_closing_brace() {
		let mut it = peekable("FOO}tail");
		let (name, modifier) = parse_braced_var(&mut it).unwrap();
		assert_eq!(name, "FOO");
		assert!(matches!(modifier, Modifier::None));
		assert_eq!(it.collect::<String>(), "tail");
	}

	#[test]
	fn parse_unterminated_var_is_none_modifier() {
		let mut it = peekable("FOO");
		let (name, modifier) = parse_braced_var(&mut it).unwrap();
		assert_eq!(name, "FOO");
		assert!(matches!(modifier, Modifier::None));
	}

	#[test]
	fn parse_each_modifier_form() {
		type Check = fn(&Modifier) -> bool;
		let cases: &[(&str, Check)] = &[
			(
				"V:-d}",
				|m| matches!(m, Modifier::DefaultIfUnsetOrEmpty(s) if s == "d"),
			),
			(
				"V-d}",
				|m| matches!(m, Modifier::DefaultIfUnset(s) if s == "d"),
			),
			(
				"V:+a}",
				|m| matches!(m, Modifier::AltIfSetAndNonEmpty(s) if s == "a"),
			),
			("V+a}", |m| matches!(m, Modifier::AltIfSet(s) if s == "a")),
			(
				"V:?e}",
				|m| matches!(m, Modifier::ErrorIfUnsetOrEmpty(s) if s == "e"),
			),
			(
				"V?e}",
				|m| matches!(m, Modifier::ErrorIfUnset(s) if s == "e"),
			),
		];
		for (input, check) in cases {
			let mut it = peekable(input);
			let (name, modifier) = parse_braced_var(&mut it).unwrap();
			assert_eq!(name, "V", "input {input}");
			assert!(check(&modifier), "modifier mismatch for {input}");
		}
	}

	#[test]
	fn parse_colon_without_known_op_defaults_to_default_if_unset_or_empty() {
		let mut it = peekable("V:x}");
		let (_, modifier) = parse_braced_var(&mut it).unwrap();
		assert!(matches!(modifier, Modifier::DefaultIfUnsetOrEmpty(s) if s == "x"));
	}

	// --- resolve_modifier ---

	#[test]
	fn resolve_none_uses_value_or_empty() {
		let v = vars(&[("A", "1")]);
		assert_eq!(
			resolve_modifier("A".into(), Modifier::None, &v).unwrap(),
			"1"
		);
		assert_eq!(
			resolve_modifier("MISSING".into(), Modifier::None, &v).unwrap(),
			""
		);
	}

	#[test]
	fn resolve_default_if_unset_or_empty() {
		let v = vars(&[("EMPTY", ""), ("SET", "x")]);
		let m = || Modifier::DefaultIfUnsetOrEmpty("def".into());
		assert_eq!(resolve_modifier("EMPTY".into(), m(), &v).unwrap(), "def");
		assert_eq!(resolve_modifier("MISSING".into(), m(), &v).unwrap(), "def");
		assert_eq!(resolve_modifier("SET".into(), m(), &v).unwrap(), "x");
	}

	#[test]
	fn resolve_default_if_unset_keeps_empty_value() {
		let v = vars(&[("EMPTY", "")]);
		assert_eq!(
			resolve_modifier("EMPTY".into(), Modifier::DefaultIfUnset("def".into()), &v).unwrap(),
			""
		);
		assert_eq!(
			resolve_modifier("MISSING".into(), Modifier::DefaultIfUnset("def".into()), &v).unwrap(),
			"def"
		);
	}

	#[test]
	fn resolve_alt_forms() {
		let v = vars(&[("EMPTY", ""), ("SET", "x")]);
		assert_eq!(
			resolve_modifier("SET".into(), Modifier::AltIfSetAndNonEmpty("a".into()), &v).unwrap(),
			"a"
		);
		assert_eq!(
			resolve_modifier(
				"EMPTY".into(),
				Modifier::AltIfSetAndNonEmpty("a".into()),
				&v
			)
			.unwrap(),
			""
		);
		assert_eq!(
			resolve_modifier("EMPTY".into(), Modifier::AltIfSet("a".into()), &v).unwrap(),
			"a"
		);
		assert_eq!(
			resolve_modifier("MISSING".into(), Modifier::AltIfSet("a".into()), &v).unwrap(),
			""
		);
	}

	#[test]
	fn resolve_error_forms() {
		let v = vars(&[("EMPTY", ""), ("SET", "x")]);
		assert!(resolve_modifier(
			"EMPTY".into(),
			Modifier::ErrorIfUnsetOrEmpty("e".into()),
			&v
		)
		.is_err());
		assert_eq!(
			resolve_modifier("SET".into(), Modifier::ErrorIfUnsetOrEmpty("e".into()), &v).unwrap(),
			"x"
		);
		assert!(
			resolve_modifier("MISSING".into(), Modifier::ErrorIfUnset("e".into()), &v).is_err()
		);
		assert_eq!(
			resolve_modifier("EMPTY".into(), Modifier::ErrorIfUnset("e".into()), &v).unwrap(),
			""
		);
	}
}
