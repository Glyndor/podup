#![no_main]

//! Fuzz the `.dockerignore` matcher. It must never panic and must answer fast:
//! an input of at most 256 bytes taking over 200 ms means the matcher went back
//! to the exponential backtracking it was rewritten to avoid. For a short
//! pattern and path it is also compared with a plain recursive matcher written
//! from the documented rules, which is slow but easy to believe.

use std::time::{Duration, Instant};

use libfuzzer_sys::fuzz_target;

const SMALL_INPUT: usize = 256;
const BUDGET: Duration = Duration::from_millis(200);
const REFERENCE_LIMIT: usize = 12;

/// `?` one byte that is not `/`; `*` a run of bytes that are not `/`; `**` any
/// run, and `**/` may also match nothing. A `**/` takes the zero-directory
/// reading only where the `**` is first reached, not after it has consumed
/// input, which is what `entered` tracks.
fn reference(pattern: &[u8], path: &[u8], entered: bool) -> bool {
	let Some((&first, rest)) = pattern.split_first() else {
		return path.is_empty();
	};
	let one_non_slash = path.first().is_some_and(|&c| c != b'/');
	if pattern.starts_with(b"**") {
		return reference(&pattern[2..], path, true)
			|| (entered && pattern.get(2) == Some(&b'/') && reference(&pattern[3..], path, true))
			|| (!path.is_empty() && reference(pattern, &path[1..], false));
	}
	match first {
		b'*' => {
			reference(rest, path, true) || (one_non_slash && reference(pattern, &path[1..], false))
		}
		b'?' => one_non_slash && reference(rest, &path[1..], true),
		literal => path.first() == Some(&literal) && reference(rest, &path[1..], true),
	}
}

/// `glob_match`'s documented front door: a pattern with no `/` and no `**` is
/// matched against the last path segment only.
fn reference_glob(pattern: &str, path: &str) -> bool {
	if !pattern.contains('/') && !pattern.contains("**") {
		let name = path.rsplit('/').next().unwrap_or(path);
		return reference(pattern.as_bytes(), name.as_bytes(), true);
	}
	reference(pattern.as_bytes(), path.as_bytes(), true)
}

fuzz_target!(|data: &[u8]| {
	let Some((&split, rest)) = data.split_first() else {
		return;
	};
	let Ok(text) = std::str::from_utf8(rest) else {
		return;
	};
	// The first byte decides where the pattern ends; the rest is the path.
	let mut cut = usize::from(split).min(text.len());
	while !text.is_char_boundary(cut) {
		cut -= 1;
	}
	let (pattern, path) = text.split_at(cut);
	// Several patterns, one per line, as a `.dockerignore` has them.
	let patterns: Vec<String> = pattern.lines().map(str::to_string).collect();

	let started = Instant::now();
	let globbed = podup::fuzz_api::glob_match(pattern, path);
	let _ = podup::fuzz_api::pattern_matches(pattern, path);
	let _ = podup::fuzz_api::evaluate(path, &patterns);
	let ignored = podup::fuzz_api::is_ignored(path, &patterns);
	let elapsed = started.elapsed();
	if data.len() <= SMALL_INPUT {
		assert!(
			elapsed <= BUDGET,
			"matching {} bytes took {elapsed:?} (budget {BUDGET:?}): pattern {pattern:?}, path {path:?}",
			data.len()
		);
	}
	// `is_ignored` is `evaluate` with `None` read as "not excluded".
	assert_eq!(
		ignored,
		podup::fuzz_api::evaluate(path, &patterns).unwrap_or(false)
	);

	if pattern.len() <= REFERENCE_LIMIT && path.len() <= REFERENCE_LIMIT {
		assert_eq!(
			globbed,
			reference_glob(pattern, path),
			"glob_match({pattern:?}, {path:?}) differs from the reference matcher"
		);
	}
});
