//! Tests for the shared `.dockerignore` matcher.
//!
//! Split out of the matcher file so the matcher stays under the source line
//! cap. The string-level cases (originally in
//! `internal/engine/build/context/pattern_tests.rs`) live here verbatim, plus
//! the spec / legacy / build-context cases that pin the watch semantics.

use crate::engine::ignore_patterns::*;

// --- string-only cases, moved from build/context/pattern_tests.rs -----------

#[test]
fn build_ignored_exact() {
	let patterns = vec!["secret.txt".to_string()];
	assert!(is_ignored("secret.txt", &patterns));
	assert!(!is_ignored("secret.txt.bak", &patterns));
}
#[test]
fn build_ignored_dir() {
	let patterns = vec!["node_modules/".to_string()];
	assert!(is_ignored("node_modules/foo.js", &patterns));
	assert!(!is_ignored("other/foo.js", &patterns));
}
#[test]
fn build_ignored_path_separator() {
	let patterns = vec!["vendor".to_string()];
	assert!(is_ignored("vendor/lib.rs", &patterns));
	assert!(!is_ignored("notvendor/lib.rs", &patterns));
}
#[test]
fn build_ignored_glob_extension() {
	let patterns = vec!["*.key".to_string()];
	assert!(is_ignored("secret.key", &patterns));
	assert!(is_ignored("certs/ca.key", &patterns));
	assert!(!is_ignored("key.txt", &patterns));
}
#[test]
fn build_ignored_glob_in_subdir() {
	let patterns = vec!["logs/*.log".to_string()];
	assert!(is_ignored("logs/error.log", &patterns));
	assert!(!is_ignored("other/error.log", &patterns));
}
#[test]
fn glob_match_star_extension() {
	assert!(glob_match("*.env", "production.env"));
	assert!(glob_match("*.env", "config/.env"));
	assert!(!glob_match("*.env", "env.txt"));
}
#[test]
fn glob_match_star_prefix() {
	assert!(glob_match("id_*", "id_rsa"));
	assert!(glob_match("id_*", "id_ed25519"));
	assert!(!glob_match("id_*", "not_id_rsa"));
}
#[test]
fn glob_match_double_star_any_depth() {
	assert!(glob_match("**/*.key", "secret.key"));
	assert!(glob_match("**/*.key", "a/b/c/secret.key"));
	assert!(glob_match("a/**/b", "a/b"));
	assert!(glob_match("a/**/b", "a/x/y/b"));
	assert!(!glob_match("a/**/b", "z/b"));
}
#[test]
fn glob_match_question_mark() {
	assert!(glob_match("file?.txt", "file1.txt"));
	assert!(!glob_match("file?.txt", "file.txt"));
	assert!(!glob_match("file?.txt", "file12.txt"));
}
#[test]
fn dockerignore_negation_reincludes() {
	let patterns = vec!["*.log".to_string(), "!keep.log".to_string()];
	assert!(is_ignored("error.log", &patterns));
	assert!(!is_ignored("keep.log", &patterns));
}
#[test]
fn dockerignore_negation_order_matters() {
	// Re-include then exclude again: last match wins.
	let patterns = vec![
		"logs/".to_string(),
		"!logs/keep/".to_string(),
		"logs/keep/secret.txt".to_string(),
	];
	assert!(is_ignored("logs/a.log", &patterns));
	assert!(!is_ignored("logs/keep/b.log", &patterns));
	assert!(is_ignored("logs/keep/secret.txt", &patterns));
}
#[test]
fn build_ignored_empty_pattern_matches_nothing() {
	// A blank `.dockerignore` line yields an empty pattern that must never
	// match (otherwise it would exclude every file).
	let patterns = vec![String::new()];
	assert!(!is_ignored("anything.txt", &patterns));
	assert!(!is_ignored("a/b/c", &patterns));
}
#[test]
fn glob_match_double_star_suffix_spans_subtree() {
	// A trailing `**` matches the directory and everything beneath it.
	assert!(glob_match("build/**", "build/out.o"));
	assert!(glob_match("build/**", "build/a/b/out.o"));
	assert!(!glob_match("build/**", "src/out.o"));
}
#[test]
fn glob_match_double_star_middle_with_no_match_fails() {
	// `a/**/z` requires the path to start with `a/` and end with `z`; a path
	// that never reaches the trailing literal exhausts the `**` prefix loop and
	// fails rather than matching loosely.
	assert!(glob_match("a/**/z", "a/b/c/z"));
	assert!(!glob_match("a/**/z", "a/b/c/y"));
}
#[test]
fn glob_match_question_mark_matches_single_non_slash_char() {
	// `?` matches exactly one character and never a path separator.
	assert!(glob_match("file?.txt", "file1.txt"));
	assert!(!glob_match("file?.txt", "file.txt"));
	assert!(!glob_match("a?b", "a/b"));
}
#[test]
fn ignore_matching_uses_forward_slashes_on_every_platform() {
	// `.dockerignore` patterns always use `/`. `Path` yields `\` on Windows,
	// so matching the raw string meant nothing below the top level was ever
	// ignored there and `vendor/` silently did nothing. The tar writer
	// already normalises, so the entry names and the ignore check disagreed
	// about the same file. Caught by the negation case failing on the
	// Windows runner and passing everywhere else.
	let rel = std::path::Path::new("vendor").join("drop.txt");
	assert_eq!(
		to_ignore_path(&rel),
		"vendor/drop.txt",
		"the ignore path must be slash-separated whatever the platform uses"
	);
	assert!(is_ignored(&to_ignore_path(&rel), &["vendor/".to_string()]));
}

// --- evaluate() -------------------------------------------------------------

#[test]
fn evaluate_reports_whether_any_pattern_matched() {
	let p = vec!["*.log".to_string()];
	assert_eq!(evaluate("a/b.log", &p), Some(true));
	assert_eq!(evaluate("a/b.txt", &p), None);
	let with_neg = vec!["*.log".to_string(), "!keep.log".to_string()];
	assert_eq!(evaluate("error.log", &with_neg), Some(true));
	assert_eq!(evaluate("keep.log", &with_neg), Some(false));
}

#[test]
fn evaluate_empty_patterns_returns_none() {
	assert_eq!(evaluate("anything", &[]), None);
}

// --- spec semantics (path relative to the rule's `path`) -------------------

fn rel_to(rule_path: &str, full: &str) -> String {
	// Mirrors what watch computes: strip the rule's `path` from the changed
	// event path. Returns the segment that should be fed to the matcher.
	let rule = std::path::Path::new(rule_path);
	let full = std::path::Path::new(full);
	full.strip_prefix(rule)
		.unwrap()
		.to_string_lossy()
		.replace('\\', "/")
}

#[test]
fn spec_ignore_dir_relative_to_rule_path() {
	let p = vec!["cache/".to_string()];
	let path = rel_to("./api", "./api/cache/x");
	assert!(is_ignored(&path, &p));
}

#[test]
fn spec_ignore_glob_extension_relative_to_rule_path() {
	let p = vec!["*.txt".to_string()];
	let path = rel_to("./api", "./api/a/b.txt");
	assert!(is_ignored(&path, &p));
	let path = rel_to("./api", "./api/src/main.rs");
	assert!(!is_ignored(&path, &p));
}

#[test]
fn spec_ignore_double_star_anywhere() {
	let p = vec!["**/cache/**".to_string()];
	let path = rel_to("./api", "./api/x/cache/y");
	assert!(is_ignored(&path, &p));
}

#[test]
fn spec_ignore_subtree_under_rule_path() {
	let p = vec!["cache/**".to_string()];
	let path = rel_to("./api", "./api/cache/y");
	assert!(is_ignored(&path, &p));
}

#[test]
fn spec_ignore_does_not_match_unrelated_paths() {
	let p = vec!["cache/".to_string()];
	let path = rel_to("./api", "./api/src/main.rs");
	assert!(!is_ignored(&path, &p));
}

#[test]
fn spec_negation_keeps_reinclude_authoritative() {
	// `cache/` then `!cache/keep`: `cache/keep` is re-included and must not
	// be ignored; `cache/other` is ignored. The "no warning" half of this
	// is enforced by the spec matcher's `Some(false)` decision, which the
	// legacy fallback (see watch) honours by skipping the fallback entirely.
	let p = vec!["cache/".to_string(), "!cache/keep".to_string()];
	let keep = rel_to("./api", "./api/cache/keep");
	let other = rel_to("./api", "./api/cache/other");
	assert_eq!(evaluate(&keep, &p), Some(false));
	assert_eq!(evaluate(&other, &p), Some(true));
	assert!(!is_ignored(&keep, &p));
	assert!(is_ignored(&other, &p));
}

// --- legacy (project-relative) fallback ------------------------------------

// Legacy cases live with the watch fix that introduces the fallback; here
// the matcher file only needs to pin its own semantics.
