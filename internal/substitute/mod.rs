//! Docker Compose variable substitution.
//!
//! Applies `${VAR}` / `$VAR` substitution to raw YAML text before parsing.
//! Handles all compose-spec modifier forms: `:-`, `-`, `:+`, `+`, `:?`, `?`.

mod parse;

use std::collections::HashMap;
use std::path::Path;

use crate::error::Result;

use parse::{collect_var_name, is_var_start, parse_braced_var, resolve_modifier};

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
/// - `KEY=VALUE` sets KEY to VALUE; surrounding quotes are stripped and
///   dotenv escapes/inline comments are handled.
/// - `KEY` without `=` sets KEY to empty string.
/// - Process environment variables take precedence: if a key already exists in
///   the current process env it will *not* be overridden by the `.env` file.
pub fn load_dotenv(dir: &Path) -> HashMap<String, String> {
	let path = dir.join(".env");
	let Ok(content) = crate::filesystem::read_to_string_capped(&path) else {
		return HashMap::new();
	};

	let mut map = HashMap::new();
	for (key, value) in crate::dotenv::parse(&content) {
		// Process environment variables take precedence over the `.env` file.
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
		let Ok(content) = crate::filesystem::read_to_string_capped(&abs) else {
			continue;
		};
		for (key, value) in crate::dotenv::parse(&content) {
			vars.entry(key).or_insert(value);
		}
	}
	vars
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
		pairs
			.iter()
			.map(|(k, v)| (k.to_string(), v.to_string()))
			.collect()
	}

	// Plain passthrough

	#[test]
	fn plain_text_unchanged() {
		assert_eq!(
			substitute("hello world", &vars(&[])).unwrap(),
			"hello world"
		);
	}

	#[test]
	fn dollar_at_end_emitted_literally() {
		assert_eq!(substitute("price$", &vars(&[])).unwrap(), "price$");
	}

	#[test]
	fn double_dollar_becomes_single() {
		assert_eq!(substitute("$$", &vars(&[])).unwrap(), "$");
	}

	// $VAR (unbraced)

	#[test]
	fn unbraced_var_set_expands() {
		assert_eq!(substitute("$FOO", &vars(&[("FOO", "bar")])).unwrap(), "bar");
	}

	#[test]
	fn unbraced_var_unset_expands_to_empty() {
		assert_eq!(substitute("$MISSING", &vars(&[])).unwrap(), "");
	}

	#[test]
	fn unbraced_var_followed_by_non_ident() {
		assert_eq!(substitute("$FOO!", &vars(&[("FOO", "x")])).unwrap(), "x!");
	}

	// ${VAR} (braced, no modifier)

	#[test]
	fn braced_var_set_expands() {
		assert_eq!(
			substitute("${FOO}", &vars(&[("FOO", "val")])).unwrap(),
			"val"
		);
	}

	#[test]
	fn braced_var_unset_expands_to_empty() {
		assert_eq!(substitute("${MISSING}", &vars(&[])).unwrap(), "");
	}

	// ${VAR:-default}

	#[test]
	fn default_if_unset_or_empty_when_unset() {
		assert_eq!(
			substitute("${X:-fallback}", &vars(&[])).unwrap(),
			"fallback"
		);
	}

	#[test]
	fn default_if_unset_or_empty_when_empty() {
		assert_eq!(
			substitute("${X:-fallback}", &vars(&[("X", "")])).unwrap(),
			"fallback"
		);
	}

	#[test]
	fn default_if_unset_or_empty_when_set() {
		assert_eq!(
			substitute("${X:-fallback}", &vars(&[("X", "real")])).unwrap(),
			"real"
		);
	}

	// ${VAR-default}

	#[test]
	fn default_if_unset_when_unset() {
		assert_eq!(substitute("${X-fallback}", &vars(&[])).unwrap(), "fallback");
	}

	#[test]
	fn default_if_unset_when_empty_keeps_empty() {
		assert_eq!(
			substitute("${X-fallback}", &vars(&[("X", "")])).unwrap(),
			""
		);
	}

	#[test]
	fn default_if_unset_when_set() {
		assert_eq!(
			substitute("${X-fallback}", &vars(&[("X", "v")])).unwrap(),
			"v"
		);
	}

	// ${VAR:+alt}

	#[test]
	fn alt_if_set_and_nonempty_when_unset() {
		assert_eq!(substitute("${X:+alt}", &vars(&[])).unwrap(), "");
	}

	#[test]
	fn alt_if_set_and_nonempty_when_empty() {
		assert_eq!(substitute("${X:+alt}", &vars(&[("X", "")])).unwrap(), "");
	}

	#[test]
	fn alt_if_set_and_nonempty_when_set() {
		assert_eq!(
			substitute("${X:+alt}", &vars(&[("X", "v")])).unwrap(),
			"alt"
		);
	}

	// ${VAR+alt}

	#[test]
	fn alt_if_set_when_unset() {
		assert_eq!(substitute("${X+alt}", &vars(&[])).unwrap(), "");
	}

	#[test]
	fn alt_if_set_when_empty_returns_alt() {
		assert_eq!(substitute("${X+alt}", &vars(&[("X", "")])).unwrap(), "alt");
	}

	// ${VAR:?msg}

	#[test]
	fn error_if_unset_or_empty_when_unset() {
		assert!(substitute("${X:?required}", &vars(&[])).is_err());
	}

	#[test]
	fn error_if_unset_or_empty_when_empty() {
		assert!(substitute("${X:?required}", &vars(&[("X", "")])).is_err());
	}

	#[test]
	fn error_if_unset_or_empty_when_set() {
		assert_eq!(
			substitute("${X:?required}", &vars(&[("X", "ok")])).unwrap(),
			"ok"
		);
	}

	// ${VAR?msg}

	#[test]
	fn error_if_unset_when_unset() {
		assert!(substitute("${X?required}", &vars(&[])).is_err());
	}

	#[test]
	fn error_if_unset_when_empty_returns_empty() {
		assert_eq!(
			substitute("${X?required}", &vars(&[("X", "")])).unwrap(),
			""
		);
	}

	// Nested interpolation inside modifier values (compose-spec allows nesting).

	#[test]
	fn nested_default_is_interpolated() {
		// ${FOO:-${BAR}} → BAR's value when FOO is unset.
		assert_eq!(
			substitute("${FOO:-${BAR}}", &vars(&[("BAR", "b")])).unwrap(),
			"b"
		);
	}

	#[test]
	fn nested_chained_default_falls_through() {
		// ${FOO:-${BAR:-baz}} → literal baz when both FOO and BAR are unset.
		assert_eq!(
			substitute("${FOO:-${BAR:-baz}}", &vars(&[])).unwrap(),
			"baz"
		);
	}

	#[test]
	fn nested_alt_is_interpolated() {
		// ${FOO:+${BAR}} → BAR's value when FOO is set and non-empty.
		assert_eq!(
			substitute("${FOO:+${BAR}}", &vars(&[("FOO", "x"), ("BAR", "b")])).unwrap(),
			"b"
		);
	}

	#[test]
	fn nested_default_with_trailing_text() {
		// The balanced-brace scan stops at the matching close, leaving following text.
		assert_eq!(
			substitute("${FOO:-${BAR}}/tail", &vars(&[("BAR", "b")])).unwrap(),
			"b/tail"
		);
	}

	// Multiple substitutions in one string

	#[test]
	fn multiple_vars_in_string() {
		let v = vars(&[("A", "hello"), ("B", "world")]);
		assert_eq!(substitute("$A ${B}!", &v).unwrap(), "hello world!");
	}

	// load_dotenv

	#[test]
	fn load_dotenv_strips_double_quoted_value() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(".env"), "FOO=\"bar\"\n").unwrap();
		let map = load_dotenv(dir.path());
		assert_eq!(map.get("FOO").map(|s| s.as_str()), Some("bar"));
	}

	#[test]
	fn load_dotenv_strips_single_quoted_value() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(".env"), "FOO='bar'\n").unwrap();
		let map = load_dotenv(dir.path());
		assert_eq!(map.get("FOO").map(|s| s.as_str()), Some("bar"));
	}

	#[test]
	fn load_dotenv_parses_key_value() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(".env"), "FOO=bar\nBAZ=qux\n").unwrap();
		let map = load_dotenv(dir.path());
		assert_eq!(map.get("FOO").map(|s| s.as_str()), Some("bar"));
		assert_eq!(map.get("BAZ").map(|s| s.as_str()), Some("qux"));
	}

	#[test]
	fn load_dotenv_skips_comments_and_blank_lines() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(".env"), "# comment\n\nFOO=bar\n").unwrap();
		let map = load_dotenv(dir.path());
		assert_eq!(map.len(), 1);
		assert_eq!(map["FOO"], "bar");
	}

	#[test]
	fn load_dotenv_key_without_equals_is_empty() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join(".env"), "BARE_KEY\n").unwrap();
		let map = load_dotenv(dir.path());
		assert_eq!(map.get("BARE_KEY").map(|s| s.as_str()), Some(""));
	}

	#[test]
	fn load_dotenv_missing_file_returns_empty() {
		let dir = tempfile::tempdir().unwrap();
		let map = load_dotenv(dir.path());
		assert!(map.is_empty());
	}

	// build_vars_with_env_files

	#[test]
	fn build_vars_with_env_files_loads_relative_extra_file() {
		let dir = tempfile::tempdir().unwrap();
		// A `.env` provides a base var; the extra file adds another and does NOT
		// override the dotenv value (process env / earlier sources win via entry).
		std::fs::write(
			dir.path().join(".env"),
			"FROM_DOTENV=base\nPODUP_TEST_SHARED=dotenv\n",
		)
		.unwrap();
		std::fs::write(
			dir.path().join("extra.env"),
			"FROM_EXTRA=more\nPODUP_TEST_SHARED=extra\n",
		)
		.unwrap();

		let vars = build_vars_with_env_files(dir.path(), &["extra.env".to_string()]);
		assert_eq!(vars.get("FROM_DOTENV").map(String::as_str), Some("base"));
		assert_eq!(vars.get("FROM_EXTRA").map(String::as_str), Some("more"));
		// The extra file must not clobber a value an earlier source already set.
		assert_eq!(
			vars.get("PODUP_TEST_SHARED").map(String::as_str),
			Some("dotenv")
		);
	}

	#[test]
	fn build_vars_with_env_files_skips_missing_extra_file() {
		let dir = tempfile::tempdir().unwrap();
		// A missing extra file is silently skipped rather than erroring.
		let vars = build_vars_with_env_files(dir.path(), &["absent.env".to_string()]);
		assert!(!vars.contains_key("FROM_EXTRA"));
	}
}
