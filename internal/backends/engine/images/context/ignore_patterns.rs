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

mod glob;

/// Read the patterns from `context`'s active ignore file.
///
/// Returns the same `(name, patterns)` pair the build tar loop has always
/// used: `name` is `.containerignore` or `.dockerignore` (the file podman
/// reads), `patterns` is the trimmed, comment-stripped, non-empty line list.
/// Exactly one file applies, never the union, matching podman-build(1).
///
/// A file that is missing, or that is not a regular file (a link out of the
/// tree is not followed), is skipped. Any other failure, such as a file that is
/// unreadable, too large or not text, is an error: carrying on without the
/// rules would ship the files the operator meant to keep out.
pub(in crate::backends::engine) fn try_read_patterns(
	context: &Path,
) -> std::io::Result<(&'static str, Vec<String>)> {
	for name in [".containerignore", ".dockerignore"] {
		let content = match crate::filesystem::read_to_string_capped(context.join(name)) {
			Ok(content) => content,
			Err(e)
				if matches!(
					e.kind(),
					std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidInput
				) =>
			{
				continue;
			}
			Err(e) => return Err(e),
		};
		let patterns = content
			.trim_start_matches('\u{feff}')
			.lines()
			.filter(|l| !l.starts_with('#'))
			.map(|l| l.trim().to_string())
			.filter(|l| !l.is_empty())
			.collect();
		return Ok((name, patterns));
	}
	Ok((".containerignore", Vec::new()))
}

/// [`try_read_patterns`] for callers that can only act on a list: a failure
/// reads as no patterns. The build does not use it.
pub(in crate::backends::engine) fn read_patterns(context: &Path) -> (&'static str, Vec<String>) {
	try_read_patterns(context).unwrap_or((".containerignore", Vec::new()))
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
pub(in crate::backends::engine) fn to_ignore_path(rel: &std::path::Path) -> String {
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
/// asked to keep. Normalize literal exceptions before comparing their paths;
/// wildcard or escaped exceptions conservatively keep descent enabled.
pub(in crate::backends::engine) fn negation_could_reach(dir: &str, patterns: &[String]) -> bool {
	patterns
		.iter()
		.filter_map(|p| p.trim().strip_prefix('!'))
		.any(|p| {
			let p = normalize(p);
			!p.is_empty()
				&& (dir.is_empty()
					|| p.contains(['*', '?', '[', '\\'])
					|| p == dir || p.strip_prefix(dir).is_some_and(|s| s.starts_with('/'))
					|| dir
						.strip_prefix(p.as_str())
						.is_some_and(|s| s.starts_with('/')))
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
pub(in crate::backends::engine) fn evaluate(path: &str, patterns: &[String]) -> Option<bool> {
	let mut last: Option<bool> = None;
	for pattern in patterns {
		let pattern = pattern.trim();
		let (negated, pat) = match pattern.strip_prefix('!') {
			Some(rest) => (true, rest.trim()),
			None => (false, pattern),
		};
		if pattern_matches(pat, path) {
			last = Some(!negated);
		}
	}
	last
}

/// Decide whether `path` is excluded from the build context.
pub(in crate::backends::engine) fn is_ignored(path: &str, patterns: &[String]) -> bool {
	evaluate(path, patterns).unwrap_or(false)
}

/// Match a single (already de-negated) `.dockerignore` pattern against `path`.
pub(in crate::backends::engine) fn pattern_matches(pattern: &str, path: &str) -> bool {
	let pattern = normalize(pattern);
	if pattern.is_empty() {
		return false;
	}
	let wildcard = pattern.contains(['*', '?', '[', '\\']);
	let matches = |candidate: &str| {
		if wildcard {
			glob_match(&pattern, candidate)
		} else {
			candidate == pattern
		}
	};
	if matches(path) {
		return true;
	}
	// A matched directory excludes its descendants, including when its name
	// was matched by a glob. Each later negation gets the same parent checks.
	path.match_indices('/')
		.any(|(end, _)| matches(&path[..end]))
}

fn normalize(pattern: &str) -> String {
	let pattern = pattern.trim();
	let absolute = pattern.starts_with('/');
	let mut parts = Vec::new();
	for part in pattern.split('/') {
		match part {
			"" | "." => {}
			".." if parts.last().is_some_and(|p| *p != "..") => {
				parts.pop();
			}
			".." if absolute => {}
			_ => parts.push(part),
		}
	}
	parts.join("/")
}

/// Match path against a glob pattern.
///
/// Patterns without `/` are matched against the filename only, so `*.log`
/// excludes both `error.log` and `logs/error.log`. A single `*` never crosses a
/// `/` boundary; `**` matches any number of path segments (including `/`), so
/// `**/*.key` and `a/**/b` work like Docker.
pub(in crate::backends::engine) fn glob_match(pattern: &str, path: &str) -> bool {
	if !pattern.contains('/') && !pattern.contains("**") {
		let filename = path.rsplit('/').next().unwrap_or(path);
		return glob_rec(pattern.as_bytes(), filename.as_bytes());
	}
	glob_rec(pattern.as_bytes(), path.as_bytes())
}

/// Glob matcher: `?` matches one non-`/` char, `*` matches any run of non-`/`
/// chars, `**` matches any run including `/`, and `**/` may also match zero
/// directories.
///
/// The pattern is run as a set of positions advanced one byte of `s` at a time,
/// so the cost is `pat.len() * s.len()` however many wildcards are stacked. The
/// earlier recursive matcher retried every split of every wildcard and took
/// exponential time on inputs such as `*a*a*a*a*a*a*b` against a run of `a`.
pub(in crate::backends::engine) fn glob_rec(pat: &[u8], s: &[u8]) -> bool {
	if !pat.is_ascii() || !s.is_ascii() || pat.contains(&b'[') || pat.contains(&b'\\') {
		return glob::matches(pat, s);
	}
	let n = pat.len();
	let mut cur = vec![false; n + 1];
	let mut stay = vec![false; n + 1];
	cur[0] = true;
	close(pat, &mut cur, &stay);
	for &c in s {
		let mut next = vec![false; n + 1];
		stay.iter_mut().for_each(|b| *b = false);
		for i in 0..n {
			if !cur[i] {
				continue;
			}
			if pat[i..].starts_with(b"**") {
				stay[i] = true;
			} else {
				match pat[i] {
					b'*' => stay[i] = c != b'/',
					b'?' => next[i + 1] |= c != b'/',
					lit => next[i + 1] |= lit == c,
				}
			}
		}
		close(pat, &mut next, &stay);
		cur = next;
	}
	cur[n]
}

/// Add the positions reachable without consuming input. `set` holds the
/// positions just entered and `stay` the wildcards that consumed this byte and
/// remain active. A `*` or `**` may match nothing; `**/` may match zero
/// directories, but only when entered, not on every byte it goes on to consume.
/// Every such move goes forward, so one pass in index order is enough.
fn close(pat: &[u8], set: &mut [bool], stay: &[bool]) {
	for i in 0..pat.len() {
		let entered = set[i];
		if !entered && !stay[i] {
			continue;
		}
		set[i] = true;
		if pat[i..].starts_with(b"**") {
			set[i + 2] = true;
			if entered && pat.get(i + 2) == Some(&b'/') {
				set[i + 3] = true;
			}
		} else if pat[i] == b'*' {
			set[i + 1] = true;
		}
	}
}
