//! Entry points for the fuzz harness. The ignore matcher is private to the
//! engine, so these forward to it unchanged and add no behaviour of their own.

use super::ignore_patterns;

/// [`ignore_patterns::glob_match`], unchanged.
pub fn glob_match(pattern: &str, path: &str) -> bool {
	ignore_patterns::glob_match(pattern, path)
}

/// [`ignore_patterns::pattern_matches`], unchanged.
pub fn pattern_matches(pattern: &str, path: &str) -> bool {
	ignore_patterns::pattern_matches(pattern, path)
}

/// [`ignore_patterns::evaluate`], unchanged.
pub fn evaluate(path: &str, patterns: &[String]) -> Option<bool> {
	ignore_patterns::evaluate(path, patterns)
}

/// [`ignore_patterns::is_ignored`], unchanged.
pub fn is_ignored(path: &str, patterns: &[String]) -> bool {
	ignore_patterns::is_ignored(path, patterns)
}
