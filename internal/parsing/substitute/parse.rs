//! `${VAR}` / `$VAR` reference parsing and modifier resolution.
//!
//! Implements the compose-spec modifier forms (`:-`, `-`, `:+`, `+`, `:?`, `?`)
//! by scanning the characters inside a `${…}` group and resolving them against
//! the variable map.

use std::collections::{HashMap, HashSet};

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
	/// `${VAR:-default}`: use default if unset or empty
	DefaultIfUnsetOrEmpty(String),
	/// `${VAR-default}`: use default if unset (empty value is OK)
	DefaultIfUnset(String),
	/// `${VAR:+value}`: use value if set and non-empty
	AltIfSetAndNonEmpty(String),
	/// `${VAR+value}`: use value if set (even if empty)
	AltIfSet(String),
	/// `${VAR:?error}`: error if unset or empty
	ErrorIfUnsetOrEmpty(String),
	/// `${VAR?error}`: error if unset
	ErrorIfUnset(String),
}

impl Modifier {
	/// The bytes of the text between the operator and the closing brace: the
	/// default, the alternative or the error message. It was copied out of the
	/// input to be parsed, and a nested reference copies its own text again.
	pub(super) fn text_len(&self) -> usize {
		match self {
			Modifier::None => 0,
			Modifier::DefaultIfUnsetOrEmpty(t)
			| Modifier::DefaultIfUnset(t)
			| Modifier::AltIfSetAndNonEmpty(t)
			| Modifier::AltIfSet(t)
			| Modifier::ErrorIfUnsetOrEmpty(t)
			| Modifier::ErrorIfUnset(t) => t.len(),
		}
	}
}

/// Parse the content inside `${…}`.  The opening `{` has already been consumed.
///
/// The variable name is collected with [`is_var_char`] (matching the unbraced
/// `$VAR` path and the compose-spec grammar). It must be non-empty and start
/// with `[A-Za-z_]`; `${}` and `${1BAD}` are rejected as malformed rather than
/// resolved to an empty string. After the name, only a modifier delimiter
/// (`}`, `:`, `-`, `+`, `?`) may follow; any other trailing character (a space
/// in `${FOO BAR}`, a dot in `${FOO.BAR}`, …) makes the reference malformed and
/// is rejected rather than folded into the lookup key.
///
/// The reference must also be closed and its modifier must be one Docker
/// Compose accepts. `${A` with no closing brace, and `${A:foo}` or `${A:=foo}`
/// with a modifier that is not `:-`, `-`, `:+`, `+`, `:?` or `?`, are refused
/// with [`ComposeError::InvalidInterpolationFormat`], the way Docker Compose
/// refuses them. The text of the offending reference and the path it was found
/// in are filled in by the caller, which knows both.
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
		// The reference ends with the input: `${A` with no closing brace is a
		// malformed reference, not a variable that resolves to its value.
		None => Err(ComposeError::InvalidInterpolationFormat {
			path: String::new(),
			template: String::new(),
		}),
		Some('}') => {
			chars.next();
			Ok((name, Modifier::None))
		}
		Some(':') => {
			chars.next();
			// Peek at what follows `:`. Only the modifiers Docker Compose
			// accepts are read as one; anything else (`:=`, `:foo`, `:`) is a
			// malformed reference and is refused instead of being taken for a
			// default value.
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
				_ => {
					return Err(ComposeError::InvalidInterpolationFormat {
						path: String::new(),
						template: String::new(),
					})
				}
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
/// A `{` in the value is text, so a value whose braces do not balance still
/// closes: compose reads `${FOO:-{}` as the default `{`. Such a value ends at
/// its last `}`.
///
/// The end of the group is found on a copy of the iterator, because an
/// unbalanced value only ends at the last `}` of the input, and the text that
/// follows the group must stay in the input for the caller.
///
/// If the input ends before any `}` is reached the reference is unterminated
/// (e.g. `${TAG:-latest` with no closing brace), which would otherwise
/// silently swallow the rest of the document as the modifier value; that is
/// reported as [`ComposeError::InvalidSubstitution`] instead.
fn collect_until_close(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<String> {
	let mut probe = chars.clone();
	let end = find_group_end(&mut probe)?;

	let mut buf = String::new();
	for c in chars.by_ref().take(end) {
		buf.push(c);
	}
	// The last character is the `}` that closed the group, not part of the
	// value.
	buf.pop();
	Ok(buf)
}

/// The number of characters the group spans, its closing `}` included.
///
/// The group ends at the first `}` that no `{` of the value claims. A value
/// whose braces do not balance has no such `}`: it ends at the last `}` that
/// closed a pair. A value with no `}` at all is unterminated.
fn find_group_end(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<usize> {
	let mut depth = 0u32;
	let mut end = 0usize;
	let mut pair_close: Option<usize> = None;
	for c in chars.by_ref() {
		end += 1;
		match c {
			'{' => depth += 1,
			'}' => {
				if depth == 0 {
					return Ok(end);
				}
				depth -= 1;
				pair_close = Some(end);
			}
			_ => {}
		}
	}
	pair_close.ok_or_else(|| {
		ComposeError::InvalidSubstitution(
			"unterminated variable substitution: missing closing '}'".to_string(),
		)
	})
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
	spent: &mut usize,
	warned: &mut HashSet<String>,
) -> Result<String> {
	let value = vars.get(&var);

	match modifier {
		Modifier::None => match value {
			Some(v) => Ok(v.clone()),
			None => {
				// Match docker compose v2, which warns on stderr before defaulting an
				// unreferenced variable to the empty string, so config typos surface.
				super::warn_unset(&var, warned);
				Ok(String::new())
			}
		},

		// Default/alt values are themselves interpolated (compose allows nesting,
		// e.g. `${FOO:-${BAR}}`), but only when actually used. `depth` bounds that
		// recursion so a pathological `${A:-${A:-…}}` chain returns an error rather
		// than overflowing the stack.
		Modifier::DefaultIfUnsetOrEmpty(default) => match value {
			Some(v) if !v.is_empty() => Ok(v.clone()),
			_ => super::substitute_depth(&default, vars, depth + 1, spent, warned),
		},

		Modifier::DefaultIfUnset(default) => match value {
			Some(v) => Ok(v.clone()),
			None => super::substitute_depth(&default, vars, depth + 1, spent, warned),
		},

		Modifier::AltIfSetAndNonEmpty(alt) => match value {
			Some(v) if !v.is_empty() => {
				super::substitute_depth(&alt, vars, depth + 1, spent, warned)
			}
			_ => Ok(String::new()),
		},

		Modifier::AltIfSet(alt) => match value {
			Some(_) => super::substitute_depth(&alt, vars, depth + 1, spent, warned),
			None => Ok(String::new()),
		},

		Modifier::ErrorIfUnsetOrEmpty(msg) => match value {
			Some(v) if !v.is_empty() => Ok(v.clone()),
			_ => Err(ComposeError::RequiredVarNotSet {
				var,
				msg: interpolate_message(&msg, vars, depth, spent, warned)?,
			}),
		},

		Modifier::ErrorIfUnset(msg) => match value {
			Some(v) => Ok(v.clone()),
			None => Err(ComposeError::RequiredVarNotSet {
				var,
				msg: interpolate_message(&msg, vars, depth, spent, warned)?,
			}),
		},
	}
}

/// Interpolate the message of a `:?` / `?` modifier.
///
/// The message is a template like any other modifier value, so
/// `${MISSING:?${B}}` reports the value of `B` and not the literal text
/// `${B}`. It is interpolated only when the error is raised, because the
/// message is unused while the variable holds a value.
fn interpolate_message(
	msg: &str,
	vars: &HashMap<String, String>,
	depth: usize,
	spent: &mut usize,
	warned: &mut HashSet<String>,
) -> Result<String> {
	super::substitute_depth(msg, vars, depth + 1, spent, warned)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
