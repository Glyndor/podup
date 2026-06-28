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
///
/// The variable name is collected with [`is_var_char`] (matching the unbraced
/// `$VAR` path and the compose-spec grammar). It must be non-empty and start
/// with `[A-Za-z_]`; `${}` and `${1BAD}` are rejected as malformed rather than
/// resolved to an empty string. After the name, only a modifier delimiter
/// (`}`, `:`, `-`, `+`, `?`) or end-of-input may follow; any other trailing
/// character (a space in `${FOO BAR}`, a dot in `${FOO.BAR}`, …) makes the
/// reference malformed and is rejected rather than folded into the lookup key.
pub(super) fn parse_braced_var(
	chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<(String, Modifier)> {
	let name = collect_var_name(chars);

	// The name must be a valid identifier: non-empty and starting with a letter
	// or `_`. `collect_var_name` accepts digits (they are valid *within* a name),
	// so an empty name or a digit-leading name only fails here.
	if name.is_empty() || !name.starts_with(is_var_start) {
		return Err(ComposeError::InvalidSubstitution(format!(
			"invalid variable name {name:?} in '${{…}}': names must start with a letter or '_'"
		)));
	}

	match chars.peek() {
		None => Ok((name, Modifier::None)),
		Some('}') => {
			chars.next();
			Ok((name, Modifier::None))
		}
		Some(':') => {
			chars.next();
			// Peek at what follows `:`.
			let modifier = match chars.peek() {
				Some('-') => {
					chars.next();
					Modifier::DefaultIfUnsetOrEmpty(collect_until_close(chars)?)
				}
				Some('+') => {
					chars.next();
					Modifier::AltIfSetAndNonEmpty(collect_until_close(chars)?)
				}
				Some('?') => {
					chars.next();
					Modifier::ErrorIfUnsetOrEmpty(collect_until_close(chars)?)
				}
				_ => Modifier::DefaultIfUnsetOrEmpty(collect_until_close(chars)?),
			};
			Ok((name, modifier))
		}
		Some('-') => {
			chars.next();
			Ok((name, Modifier::DefaultIfUnset(collect_until_close(chars)?)))
		}
		Some('+') => {
			chars.next();
			Ok((name, Modifier::AltIfSet(collect_until_close(chars)?)))
		}
		Some('?') => {
			chars.next();
			Ok((name, Modifier::ErrorIfUnset(collect_until_close(chars)?)))
		}
		Some(&c) => Err(ComposeError::InvalidSubstitution(format!(
			"unexpected character {c:?} in variable name '${{{name}…}}'"
		))),
	}
}

/// Collect the modifier value up to the matching closing `}` (consumed),
/// balancing nested braces so an inner `${…}` is captured whole. For
/// `${FOO:-${BAR}}` the default is `${BAR}` (not `${BAR`), enabling nested
/// interpolation in [`resolve_modifier`].
///
/// If the input ends before the matching `}` is reached the reference is
/// unterminated (e.g. `${TAG:-latest` with no closing brace), which would
/// otherwise silently swallow the rest of the document as the modifier value;
/// that is reported as [`ComposeError::InvalidSubstitution`] instead.
fn collect_until_close(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<String> {
	let mut buf = String::new();
	let mut depth = 0u32;
	for c in chars.by_ref() {
		match c {
			'{' => {
				depth += 1;
				buf.push(c);
			}
			'}' if depth == 0 => return Ok(buf),
			'}' => {
				depth -= 1;
				buf.push(c);
			}
			_ => buf.push(c),
		}
	}
	Err(ComposeError::InvalidSubstitution(
		"unterminated variable substitution: missing closing '}'".to_string(),
	))
}

/// Apply a parsed `Modifier` to `var`, implementing compose's
/// `${VAR:-default}` / `${VAR-default}` / `${VAR:+alt}` / `${VAR+alt}` /
/// `${VAR:?err}` / `${VAR?err}` substitution semantics (the `:` forms treat an
/// empty value like unset). `Modifier::None` returns the value or an empty
/// string; the `Error*` variants fail when the condition is unmet.
pub(super) fn resolve_modifier(
	var: String,
	modifier: Modifier,
	vars: &HashMap<String, String>,
	depth: usize,
) -> Result<String> {
	let value = vars.get(&var);

	match modifier {
		Modifier::None => match value {
			Some(v) => Ok(v.clone()),
			None => {
				// Match docker compose v2, which warns on stderr before defaulting an
				// unreferenced variable to the empty string, so config typos surface.
				tracing::warn!("The {var} variable is not set. Defaulting to a blank string.");
				Ok(String::new())
			}
		},

		// Default/alt values are themselves interpolated (compose allows nesting,
		// e.g. `${FOO:-${BAR}}`), but only when actually used. `depth` bounds that
		// recursion so a pathological `${A:-${A:-…}}` chain returns an error rather
		// than overflowing the stack.
		Modifier::DefaultIfUnsetOrEmpty(default) => match value {
			Some(v) if !v.is_empty() => Ok(v.clone()),
			_ => super::substitute_depth(&default, vars, depth + 1),
		},

		Modifier::DefaultIfUnset(default) => match value {
			Some(v) => Ok(v.clone()),
			None => super::substitute_depth(&default, vars, depth + 1),
		},

		Modifier::AltIfSetAndNonEmpty(alt) => match value {
			Some(v) if !v.is_empty() => super::substitute_depth(&alt, vars, depth + 1),
			_ => Ok(String::new()),
		},

		Modifier::AltIfSet(alt) => match value {
			Some(_) => super::substitute_depth(&alt, vars, depth + 1),
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

	// --- parse_braced_var: name validation ---

	#[test]
	fn parse_valid_braced_var() {
		let mut it = peekable("FOO}");
		let (name, modifier) = parse_braced_var(&mut it).unwrap();
		assert_eq!(name, "FOO");
		assert!(matches!(modifier, Modifier::None));
	}

	#[test]
	fn parse_valid_braced_var_with_default() {
		let mut it = peekable("FOO:-default}");
		let (name, modifier) = parse_braced_var(&mut it).unwrap();
		assert_eq!(name, "FOO");
		assert!(matches!(modifier, Modifier::DefaultIfUnsetOrEmpty(s) if s == "default"));
	}

	#[test]
	fn parse_braced_var_rejects_space_in_name() {
		// `${FOO BAR}` must not produce a lookup key containing a space.
		let mut it = peekable("FOO BAR}");
		let err = parse_braced_var(&mut it).expect_err("space in name must be rejected");
		assert!(
			matches!(err, ComposeError::InvalidSubstitution(_)),
			"{err:?}"
		);
	}

	#[test]
	fn parse_braced_var_rejects_dot_in_name() {
		// `${FOO.BAR}` must not produce a lookup key containing a dot.
		let mut it = peekable("FOO.BAR}");
		let err = parse_braced_var(&mut it).expect_err("dot in name must be rejected");
		assert!(
			matches!(err, ComposeError::InvalidSubstitution(_)),
			"{err:?}"
		);
	}

	#[test]
	fn parse_braced_var_rejects_empty_name() {
		// `${}` has no name and must be rejected, not resolved to an empty string.
		let mut it = peekable("}");
		let err = parse_braced_var(&mut it).expect_err("empty name must be rejected");
		assert!(
			matches!(err, ComposeError::InvalidSubstitution(_)),
			"{err:?}"
		);
	}

	#[test]
	fn parse_braced_var_rejects_digit_leading_name() {
		// `${1BAD}` is not a valid identifier (must start with a letter or `_`).
		let mut it = peekable("1BAD}");
		let err = parse_braced_var(&mut it).expect_err("digit-leading name must be rejected");
		assert!(
			matches!(err, ComposeError::InvalidSubstitution(_)),
			"{err:?}"
		);
	}

	#[test]
	fn parse_braced_var_underscore_name_is_valid() {
		// A leading underscore is a valid identifier start.
		let mut it = peekable("_FOO}");
		let (name, modifier) = parse_braced_var(&mut it).unwrap();
		assert_eq!(name, "_FOO");
		assert!(matches!(modifier, Modifier::None));
	}

	#[test]
	fn parse_unterminated_modifier_is_error() {
		// `${TAG:-latest` (no closing `}`) must not swallow the rest of the input as
		// the default value — it is reported as a malformed substitution.
		let mut it = peekable("TAG:-latest\nmore: data\n");
		let err = parse_braced_var(&mut it).expect_err("missing close brace must error");
		assert!(
			matches!(err, ComposeError::InvalidSubstitution(_)),
			"{err:?}"
		);
	}

	// --- resolve_modifier ---

	#[test]
	fn resolve_none_uses_value_or_empty() {
		let v = vars(&[("A", "1")]);
		assert_eq!(
			resolve_modifier("A".into(), Modifier::None, &v, 0).unwrap(),
			"1"
		);
		assert_eq!(
			resolve_modifier("MISSING".into(), Modifier::None, &v, 0).unwrap(),
			""
		);
	}

	#[test]
	fn resolve_default_if_unset_or_empty() {
		let v = vars(&[("EMPTY", ""), ("SET", "x")]);
		let m = || Modifier::DefaultIfUnsetOrEmpty("def".into());
		assert_eq!(resolve_modifier("EMPTY".into(), m(), &v, 0).unwrap(), "def");
		assert_eq!(
			resolve_modifier("MISSING".into(), m(), &v, 0).unwrap(),
			"def"
		);
		assert_eq!(resolve_modifier("SET".into(), m(), &v, 0).unwrap(), "x");
	}

	#[test]
	fn resolve_default_if_unset_keeps_empty_value() {
		let v = vars(&[("EMPTY", "")]);
		assert_eq!(
			resolve_modifier(
				"EMPTY".into(),
				Modifier::DefaultIfUnset("def".into()),
				&v,
				0
			)
			.unwrap(),
			""
		);
		assert_eq!(
			resolve_modifier(
				"MISSING".into(),
				Modifier::DefaultIfUnset("def".into()),
				&v,
				0
			)
			.unwrap(),
			"def"
		);
	}

	#[test]
	fn resolve_alt_forms() {
		let v = vars(&[("EMPTY", ""), ("SET", "x")]);
		assert_eq!(
			resolve_modifier(
				"SET".into(),
				Modifier::AltIfSetAndNonEmpty("a".into()),
				&v,
				0
			)
			.unwrap(),
			"a"
		);
		assert_eq!(
			resolve_modifier(
				"EMPTY".into(),
				Modifier::AltIfSetAndNonEmpty("a".into()),
				&v,
				0
			)
			.unwrap(),
			""
		);
		assert_eq!(
			resolve_modifier("EMPTY".into(), Modifier::AltIfSet("a".into()), &v, 0).unwrap(),
			"a"
		);
		assert_eq!(
			resolve_modifier("MISSING".into(), Modifier::AltIfSet("a".into()), &v, 0).unwrap(),
			""
		);
	}

	#[test]
	fn resolve_error_forms() {
		let v = vars(&[("EMPTY", ""), ("SET", "x")]);
		assert!(resolve_modifier(
			"EMPTY".into(),
			Modifier::ErrorIfUnsetOrEmpty("e".into()),
			&v,
			0
		)
		.is_err());
		assert_eq!(
			resolve_modifier(
				"SET".into(),
				Modifier::ErrorIfUnsetOrEmpty("e".into()),
				&v,
				0
			)
			.unwrap(),
			"x"
		);
		assert!(
			resolve_modifier("MISSING".into(), Modifier::ErrorIfUnset("e".into()), &v, 0).is_err()
		);
		assert_eq!(
			resolve_modifier("EMPTY".into(), Modifier::ErrorIfUnset("e".into()), &v, 0).unwrap(),
			""
		);
	}
}
