//! Shared `.dockerignore` / `.containerignore` matcher.
//!
//! Both the build-context tar loop and the watch engine evaluate the same
//! pattern list against paths; the rules live here so the two callers cannot
//! drift. The matcher follows `.dockerignore` semantics:
//!
//! - patterns are evaluated in order, **last match wins** (a leading `!`
//!   re-includes a path that an earlier pattern excluded)
//! - patterns use `/` as a separator regardless of the host's path separator
//! - `*` and `?` match a single path segment; `**` matches any number of
//!   segments, including `/`
//!
//! [`is_ignored`] returns the final decision for a path; [`evaluate`] is the
//! same logic plus the "did any pattern match" signal the watch engine needs
//! to keep a negation authoritative when the legacy project-relative fallback
//! runs alongside.

use std::path::Path;

/// Read the patterns from `context`'s active ignore file.
///
/// Returns the same `(name, patterns)` pair the build tar loop has always
/// used: `name` is `.containerignore` or `.dockerignore` (the file podman
/// reads), `patterns` is the trimmed, comment-stripped, non-empty line list.
/// Exactly one file applies, never the union, matching podman-build(1).
pub(in crate::engine) fn read_patterns(context: &Path) -> (&'static str, Vec<String>) {
	for name in [".containerignore", ".dockerignore"] {
		let Ok(content) = crate::filesystem::read_to_string_capped(context.join(name)) else {
			continue;
		};
		let patterns = content
			.lines()
			.map(|l| l.trim().to_string())
			.filter(|l| !l.is_empty() && !l.starts_with('#'))
			.collect();
		return (name, patterns);
	}
	(".containerignore", Vec::new())
}

/// Decide whether `path` is excluded from the build context.
///
/// Patterns are evaluated in order and the **last** match wins, matching Docker
/// `.dockerignore` semantics: a leading `!` re-includes a path that an earlier
/// pattern excluded. So `*.log` then `!keep.log` ignores every log except
/// `keep.log`.
/// A relative path in the form `.dockerignore` patterns are written in.
///
/// Patterns always use `/`, and `Path` on Windows yields `\`, so matching the
/// raw string meant no pattern below the top level ever matched there:
/// `vendor/` silently ignored nothing. The tar writer already normalises, so
/// the entry names and the ignore check disagreed about the same file. Found
/// by a negation test failing on the Windows runner only.
pub(in crate::engine) fn to_ignore_path(rel: &std::path::Path) -> String {
	let s = rel.to_string_lossy();
	if std::path::MAIN_SEPARATOR == '/' {
		s.into_owned()
	} else {
		s.replace(std::path::MAIN_SEPARATOR, "/")
	}
}

/// Could any negation pattern re-include something under `dir`?
///
/// Conservative on purpose: a `true` costs a descent that the leaf filter
/// then throws away, while a wrong `false` silently drops a file the user
/// asked to keep. A negation whose path starts at `dir`, or that begins
/// with a wildcard and so could match at any depth, counts as reaching it.
pub(in crate::engine) fn negation_could_reach(dir: &str, patterns: &[String]) -> bool {
	patterns
		.iter()
		.filter_map(|p| p.strip_prefix('!'))
		.any(|p| {
			let p = p.trim_start_matches("./");
			p.starts_with('*') || p.starts_with(dir) || dir.is_empty()
		})
}

/// `Some(true)` if `path` is excluded by `patterns` (a positive pattern
/// matched last), `Some(false)` if `path` is re-included (a leading `!`
/// pattern matched last), `None` if no pattern matched at all.
///
/// `None` is the signal a caller needs to keep a `!` negation authoritative
/// when running an older project-relative matcher as a fallback: a `!`
/// pattern that matched must not be silently overridden by the older rule
/// deciding the path should be ignored.
pub(in crate::engine) fn evaluate(path: &str, patterns: &[String]) -> Option<bool> {
	let mut last: Option<bool> = None;
	for pattern in patterns {
		let (negated, pat) = match pattern.strip_prefix('!') {
			Some(rest) => (true, rest),
			None => (false, pattern.as_str()),
		};
		if pattern_matches(pat, path) {
			last = Some(!negated);
		}
	}
	last
}

/// Decide whether `path` is excluded from the build context.
pub(in crate::engine) fn is_ignored(path: &str, patterns: &[String]) -> bool {
	evaluate(path, patterns).unwrap_or(false)
}

/// Match a single (already de-negated) `.dockerignore` pattern against `path`.
pub(in crate::engine) fn pattern_matches(pattern: &str, path: &str) -> bool {
	if pattern.is_empty() {
		return false;
	}
	// Directory pattern (`foo/`): match the directory and everything beneath it.
	if let Some(dir) = pattern.strip_suffix('/') {
		return path == dir || path.starts_with(&format!("{dir}/"));
	}
	if pattern.contains('*') || pattern.contains('?') {
		return glob_match(pattern, path);
	}
	// Plain pattern: exact match, or a path segment prefix (`vendor` matches
	// `vendor/lib.rs`).
	path == pattern
		|| (path.starts_with(pattern) && path.as_bytes().get(pattern.len()) == Some(&b'/'))
}

/// Match path against a glob pattern.
///
/// Patterns without `/` are matched against the filename only, so `*.log`
/// excludes both `error.log` and `logs/error.log`. A single `*` never crosses a
/// `/` boundary; `**` matches any number of path segments (including `/`), so
/// `**/*.key` and `a/**/b` work like Docker.
pub(in crate::engine) fn glob_match(pattern: &str, path: &str) -> bool {
	if !pattern.contains('/') && !pattern.contains("**") {
		let filename = path.rsplit('/').next().unwrap_or(path);
		return glob_rec(pattern.as_bytes(), filename.as_bytes());
	}
	glob_rec(pattern.as_bytes(), path.as_bytes())
}

/// Backtracking glob matcher: `?` matches one non-`/` char, `*` matches any run
/// of non-`/` chars, `**` matches any run including `/`.
pub(in crate::engine) fn glob_rec(pat: &[u8], s: &[u8]) -> bool {
	if pat.is_empty() {
		return s.is_empty();
	}
	// `**` matches across `/` boundaries.
	if pat.starts_with(b"**") {
		let mut rest = &pat[2..];
		// `**/` may also match zero directories, so `**/foo` matches top-level `foo`.
		if rest.first() == Some(&b'/') && glob_rec(&rest[1..], s) {
			return true;
		}
		if rest.is_empty() {
			rest = b"";
		}
		// Try consuming any prefix of `s` (including `/`).
		for i in 0..=s.len() {
			if glob_rec(rest, &s[i..]) {
				return true;
			}
		}
		return false;
	}
	match pat[0] {
		b'*' => {
			// Match any run of non-`/` chars.
			let mut i = 0;
			loop {
				if glob_rec(&pat[1..], &s[i..]) {
					return true;
				}
				if i >= s.len() || s[i] == b'/' {
					return false;
				}
				i += 1;
			}
		}
		b'?' => !s.is_empty() && s[0] != b'/' && glob_rec(&pat[1..], &s[1..]),
		c => !s.is_empty() && s[0] == c && glob_rec(&pat[1..], &s[1..]),
	}
}
