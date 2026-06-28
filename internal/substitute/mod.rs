//! Docker Compose variable substitution.
//!
//! Applies `${VAR}` / `$VAR` substitution to individual scalar values of a
//! parsed compose document (compose-spec value-level interpolation).
//! Handles all compose-spec modifier forms: `:-`, `-`, `:+`, `+`, `:?`, `?`.

mod parse;

use std::collections::HashMap;
use std::path::Path;

use crate::error::{ComposeError, Result};

use parse::{collect_var_name, is_var_start, parse_braced_var, resolve_modifier};

/// Maximum nesting depth for interpolated default/alternate values
/// (`${A:-${A:-…}}`). Real compose files nest a handful of levels at most; this
/// cap turns a pathological chain into a clean error instead of a stack overflow.
const MAX_INTERP_DEPTH: usize = 64;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Substitute all `$VAR` / `${VAR}` references in `input` using `vars`.
///
/// `vars` should contain both the process environment and the `.env` file
/// entries (process environment takes precedence).
pub fn substitute(input: &str, vars: &HashMap<String, String>) -> Result<String> {
	substitute_depth(input, vars, 0)
}

/// Inner substitution carrying the current nesting `depth` so recursive
/// interpolation of modifier defaults/alternates (`${A:-${B}}`) is bounded.
pub(super) fn substitute_depth(
	input: &str,
	vars: &HashMap<String, String>,
	depth: usize,
) -> Result<String> {
	if depth > MAX_INTERP_DEPTH {
		return Err(ComposeError::InvalidSubstitution(format!(
			"interpolation nesting too deep (more than {MAX_INTERP_DEPTH} levels)"
		)));
	}

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
				let value = resolve_modifier(var, modifier, vars, depth)?;
				out.push_str(&value);
			}
			Some(c) if is_var_start(*c) => {
				let var = collect_var_name(&mut chars);
				let value = match vars.get(&var) {
					Some(v) => v.clone(),
					None => {
						// Match docker compose v2: warn before defaulting to blank.
						tracing::warn!(
							"The {var} variable is not set. Defaulting to a blank string."
						);
						String::new()
					}
				};
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

/// Build vars, layering explicit `--env-file` files over the process environment.
///
/// Compose v2 semantics: when one or more `--env-file` are given they *replace*
/// the default `.env` (which is therefore not loaded), and among several files
/// the **last** one wins. With no explicit files this is just [`build_vars`]
/// (process env + `.env`). Process env always takes precedence over file values.
///
/// A missing, unreadable, or malformed `--env-file` is silently skipped here
/// (legacy lenient behaviour). This signature is part of the published library
/// API and is kept for backward compatibility; the CLI drives
/// [`build_vars_with_env_files_strict`], which fails loudly on a bad file.
pub fn build_vars_with_env_files(dir: &Path, extra: &[String]) -> HashMap<String, String> {
	// `strict = false` can never produce an error.
	build_vars_with_env_files_inner(dir, extra, false).unwrap_or_default()
}

/// Like [`build_vars_with_env_files`] but rejects a bad `--env-file`.
///
/// An explicitly-passed `--env-file` that is missing, unreadable, or malformed
/// is a hard error (matching docker compose, which fails on a not-found env
/// file) rather than being silently skipped — a typo'd path must not fall back
/// to process-env/defaults and exit 0.
pub fn build_vars_with_env_files_strict(
	dir: &Path,
	extra: &[String],
) -> Result<HashMap<String, String>> {
	build_vars_with_env_files_inner(dir, extra, true)
}

fn build_vars_with_env_files_inner(
	dir: &Path,
	extra: &[String],
	strict: bool,
) -> Result<HashMap<String, String>> {
	if extra.is_empty() {
		return Ok(build_vars(dir));
	}

	// Explicit `--env-file`s replace `.env`; a later file overrides an earlier one.
	let mut file_vars: HashMap<String, String> = HashMap::new();
	for path in extra {
		let abs = if std::path::Path::new(path).is_absolute() {
			std::path::PathBuf::from(path)
		} else {
			dir.join(path)
		};
		let content = match crate::filesystem::read_to_string_capped(&abs) {
			Ok(content) => content,
			Err(e) => {
				if strict {
					return Err(crate::error::ComposeError::EnvFile(format!(
						"env file not found: {} ({e})",
						abs.display()
					)));
				}
				continue;
			}
		};
		let pairs = if strict {
			crate::dotenv::parse_strict(&content)?
		} else {
			crate::dotenv::parse(&content)
		};
		for (key, value) in pairs {
			file_vars.insert(key, value);
		}
	}

	// Process env wins over every file value.
	let mut vars: HashMap<String, String> = std::env::vars().collect();
	for (k, v) in file_vars {
		vars.entry(k).or_insert(v);
	}
	Ok(vars)
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

	// Malformed / pathological references

	#[test]
	fn empty_name_is_error() {
		assert!(substitute("${}", &vars(&[])).is_err());
	}

	#[test]
	fn digit_leading_name_is_error() {
		assert!(substitute("${1BAD}", &vars(&[])).is_err());
	}

	#[test]
	fn unterminated_modifier_is_error() {
		// The missing `}` must error rather than consume the rest of the input.
		assert!(substitute("${TAG:-latest\nmore", &vars(&[])).is_err());
	}

	#[test]
	fn deeply_nested_defaults_error_instead_of_overflowing() {
		// A pathological `${A:-${A:-…}}` chain (all default branches taken, A unset)
		// returns a clean error past the depth cap rather than overflowing the stack.
		let depth = MAX_INTERP_DEPTH + 50;
		let mut s = String::new();
		for _ in 0..depth {
			s.push_str("${A:-");
		}
		s.push('x');
		for _ in 0..depth {
			s.push('}');
		}
		let err = substitute(&s, &vars(&[])).expect_err("over-deep nesting must error");
		assert!(matches!(
			err,
			crate::error::ComposeError::InvalidSubstitution(_)
		));
	}

	#[test]
	fn moderate_nesting_still_resolves() {
		// Well within the cap, nested defaults resolve normally.
		assert_eq!(
			substitute("${A:-${B:-${C:-deep}}}", &vars(&[])).unwrap(),
			"deep"
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
	fn load_dotenv_bare_key_is_not_set_to_empty() {
		// A bare key (no `=`) no longer becomes an empty string. In `.env` it
		// resolves from the host, but `load_dotenv` already drops host-present
		// keys (process env wins), so either way a bare key never lands in the map
		// as `""` — it's host-provided for interpolation or absent.
		std::env::set_var("PODUP_DOTENV_ENV_PRESENT", "h");
		std::env::remove_var("PODUP_DOTENV_ENV_ABSENT");
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join(".env"),
			"PODUP_DOTENV_ENV_PRESENT\nPODUP_DOTENV_ENV_ABSENT\n",
		)
		.unwrap();
		let map = load_dotenv(dir.path());
		// host-present → dropped (process env wins); host-absent → omitted. Never "".
		assert!(!map.contains_key("PODUP_DOTENV_ENV_PRESENT"));
		assert!(!map.contains_key("PODUP_DOTENV_ENV_ABSENT"));
		std::env::remove_var("PODUP_DOTENV_ENV_PRESENT");
	}

	#[test]
	fn load_dotenv_missing_file_returns_empty() {
		let dir = tempfile::tempdir().unwrap();
		let map = load_dotenv(dir.path());
		assert!(map.is_empty());
	}

	// build_vars_with_env_files

	#[test]
	fn env_file_replaces_dotenv() {
		let dir = tempfile::tempdir().unwrap();
		// Compose v2: an explicit `--env-file` replaces the default `.env`, so the
		// dotenv-only key is absent and the extra file's value wins for a shared key.
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

		let vars =
			build_vars_with_env_files_strict(dir.path(), &["extra.env".to_string()]).unwrap();
		assert_eq!(vars.get("FROM_DOTENV"), None);
		assert_eq!(vars.get("FROM_EXTRA").map(String::as_str), Some("more"));
		assert_eq!(
			vars.get("PODUP_TEST_SHARED").map(String::as_str),
			Some("extra")
		);
	}

	#[test]
	fn later_env_file_wins() {
		let dir = tempfile::tempdir().unwrap();
		// Among several `--env-file`s the last one listed wins.
		std::fs::write(dir.path().join("a.env"), "FROM_A=a\nSHARED=a\n").unwrap();
		std::fs::write(dir.path().join("b.env"), "FROM_B=b\nSHARED=b\n").unwrap();

		let vars = build_vars_with_env_files_strict(
			dir.path(),
			&["a.env".to_string(), "b.env".to_string()],
		)
		.unwrap();
		assert_eq!(vars.get("FROM_A").map(String::as_str), Some("a"));
		assert_eq!(vars.get("FROM_B").map(String::as_str), Some("b"));
		assert_eq!(vars.get("SHARED").map(String::as_str), Some("b"));
	}

	#[test]
	fn process_env_wins_over_env_file() {
		let dir = tempfile::tempdir().unwrap();
		std::env::set_var("PODUP_ENVFILE_PROCESS_WINS", "from-process");
		std::fs::write(
			dir.path().join("x.env"),
			"PODUP_ENVFILE_PROCESS_WINS=from-file\n",
		)
		.unwrap();

		let vars = build_vars_with_env_files_strict(dir.path(), &["x.env".to_string()]).unwrap();
		assert_eq!(
			vars.get("PODUP_ENVFILE_PROCESS_WINS").map(String::as_str),
			Some("from-process")
		);
		std::env::remove_var("PODUP_ENVFILE_PROCESS_WINS");
	}

	#[test]
	fn no_env_file_loads_dotenv() {
		let dir = tempfile::tempdir().unwrap();
		// With no `--env-file`, `.env` is loaded as before.
		std::fs::write(dir.path().join(".env"), "FROM_DOTENV=base\n").unwrap();
		let vars = build_vars_with_env_files_strict(dir.path(), &[]).unwrap();
		assert_eq!(vars.get("FROM_DOTENV").map(String::as_str), Some("base"));
	}

	#[test]
	fn strict_build_vars_errors_on_missing_extra_file() {
		let dir = tempfile::tempdir().unwrap();
		// An explicitly-passed `--env-file` that does not exist is a hard error
		// (docker compose parity), not a silent fall-back to defaults.
		let err =
			build_vars_with_env_files_strict(dir.path(), &["absent.env".to_string()]).unwrap_err();
		assert!(
			matches!(err, crate::error::ComposeError::EnvFile(_)),
			"expected EnvFile error, got {err:?}"
		);
		assert!(err.to_string().contains("env file not found"));
	}

	#[test]
	fn strict_build_vars_errors_on_unterminated_quote() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("bad.env"), "A=\"oops\nB=keep\n").unwrap();
		let err =
			build_vars_with_env_files_strict(dir.path(), &["bad.env".to_string()]).unwrap_err();
		assert!(matches!(err, crate::error::ComposeError::EnvFile(_)));
	}

	#[test]
	fn lenient_build_vars_skips_missing_extra_file() {
		let dir = tempfile::tempdir().unwrap();
		// The backward-compatible shim never errors: a missing extra file is
		// silently skipped rather than failing.
		let vars = build_vars_with_env_files(dir.path(), &["absent.env".to_string()]);
		assert!(!vars.contains_key("FROM_EXTRA"));
	}
}
