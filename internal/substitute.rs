//! Docker Compose variable substitution.
//!
//! Applies `${VAR}` / `$VAR` substitution to raw YAML text before parsing.
//! Handles all compose-spec modifier forms: `:-`, `-`, `:+`, `+`, `:?`, `?`.

use std::collections::HashMap;
use std::path::Path;

use crate::error::{ComposeError, Result};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Substitute all `$VAR` / `${VAR}` references in `input` using `vars`.
///
/// `vars` should contain both the process environment and the `.env` file
/// entries (process environment takes precedence).
pub fn substitute(input: &str, vars: &HashMap<String, String>) -> Result<String> {
	let mut out = String::with_capacity(input.len());
	let mut chars = input.chars().peekable();

	while let Some(ch) = chars.next() {
		if ch != '$' {
			out.push(ch);
			continue;
		}

		match chars.peek() {
			None => {
				out.push('$');
			}
			Some('$') => {
				chars.next();
				out.push('$');
			}
			Some('{') => {
				chars.next();
				let (var, modifier) = parse_braced_var(&mut chars)?;
				let value = resolve_modifier(var, modifier, vars)?;
				out.push_str(&value);
			}
			Some(c) if is_var_start(*c) => {
				let var = collect_var_name(&mut chars);
				let value = vars.get(&var).cloned().unwrap_or_default();
				out.push_str(&value);
			}
			Some(_) => {
				out.push('$');
			}
		}
	}

	Ok(out)
}

/// Load a `.env` file from `dir`.
///
/// - Lines starting with `#` are comments and are skipped.
/// - Empty / whitespace-only lines are skipped.
/// - `KEY=VALUE` sets KEY to VALUE (quotes are preserved as-is, matching compose-spec).
/// - `KEY` without `=` sets KEY to empty string.
/// - Process environment variables take precedence: if a key already exists in
///   the current process env it will *not* be overridden by the `.env` file.
pub fn load_dotenv(dir: &Path) -> HashMap<String, String> {
	let path = dir.join(".env");
	let Ok(content) = std::fs::read_to_string(&path) else {
		return HashMap::new();
	};

	let mut map = HashMap::new();
	for line in content.lines() {
		let trimmed = line.trim();
		if trimmed.is_empty() || trimmed.starts_with('#') {
			continue;
		}
		let (key, value) = if let Some(eq) = trimmed.find('=') {
			let k = trimmed[..eq].trim().to_string();
			let v = trimmed[eq + 1..].to_string();
			(k, v)
		} else {
			(trimmed.to_string(), String::new())
		};

		if key.is_empty() {
			continue;
		}

		if std::env::var(&key).is_ok() {
			continue;
		}

		map.insert(key, value);
	}

	map
}

/// Build the full variable map: process env + dotenv (process env wins).
pub fn build_vars(dir: &Path) -> HashMap<String, String> {
	let mut vars: HashMap<String, String> = std::env::vars().collect();
	for (k, v) in load_dotenv(dir) {
		vars.entry(k).or_insert(v);
	}
	vars
}

/// Build vars additionally loading explicit env files (process env + dotenv + extra files).
///
/// Extra files are loaded after dotenv; process env still wins for all keys.
pub fn build_vars_with_env_files(dir: &Path, extra: &[String]) -> HashMap<String, String> {
	let mut vars = build_vars(dir);
	for path in extra {
		let abs = if std::path::Path::new(path).is_absolute() {
			std::path::PathBuf::from(path)
		} else {
			dir.join(path)
		};
		let Ok(content) = std::fs::read_to_string(&abs) else {
			continue;
		};
		for line in content.lines() {
			let trimmed = line.trim();
			if trimmed.is_empty() || trimmed.starts_with('#') {
				continue;
			}
			let (key, value) = if let Some(eq) = trimmed.find('=') {
				(
					trimmed[..eq].trim().to_string(),
					trimmed[eq + 1..].to_string(),
				)
			} else {
				(trimmed.to_string(), String::new())
			};
			if !key.is_empty() {
				vars.entry(key).or_insert(value);
			}
		}
	}
	vars
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn is_var_start(c: char) -> bool {
	c.is_alphabetic() || c == '_'
}

fn is_var_char(c: char) -> bool {
	c.is_alphanumeric() || c == '_'
}

fn collect_var_name(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
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
enum Modifier {
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
fn parse_braced_var(
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

fn resolve_modifier(
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
