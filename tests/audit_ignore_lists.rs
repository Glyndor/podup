//! `cargo audit` and `cargo deny` agree on which RUSTSEC ids are ignored.
//!
//! `deny.toml` carries the cargo-deny ignore list and `.cargo/audit.toml`
//! carries the cargo-audit one. `cargo-deny` reads `deny.toml`;
//! `cargo-audit` reads `.cargo/audit.toml`, relative to its working
//! directory. The release job (release.yml) runs `cargo audit --deny
//! warnings` from the repository root, so an ignore that lives only in
//! deny.toml does not help the release gate: the next release would fail
//! on the same advisory that turned the scheduled audit red on
//! 2026-10-12 (#2034). The two lists must be equal.
//!
//! One parser reads both files. It strips `#`-style comments (only
//! outside double-quoted strings, so `reason = "see #2035"` survives),
//! walks until the `[advisories]` table header, takes everything between
//! its `ignore = [` and the matching `]`, and collects every
//! double-quoted string whose WHOLE value matches
//! `^RUSTSEC-[0-9]{4}-[0-9]{4}$`. The two shapes cargo-deny accepts,
//! `{ id = "X", reason = "..." }` and the plain string `"X"`, resolve to
//! the same set; an id that only appears inside a longer string is not
//! collected.

use std::fs;
use std::path::Path;

fn read_repo(rel: &str) -> String {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
	fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel} is readable: {e}"))
}

/// Strip `#`-prefixed comments from a TOML-like file, leaving any `#`
/// that lives inside a `"..."` string alone. Each returned line still
/// contains its indentation and any trailing content, so the parser that
/// consumes this output does not lose column positions.
fn strip_comments(body: &str) -> String {
	let mut out = String::with_capacity(body.len());
	let bytes = body.as_bytes();
	let mut i = 0;
	let mut at_line_start = true;
	while i < bytes.len() {
		let c = bytes[i];
		if c == b'"' {
			out.push('"');
			i += 1;
			while i < bytes.len() && bytes[i] != b'"' {
				out.push(bytes[i] as char);
				i += 1;
			}
			if i < bytes.len() {
				out.push('"');
				i += 1;
			}
			at_line_start = false;
			continue;
		}
		if at_line_start && c == b'#' {
			while i < bytes.len() && bytes[i] != b'\n' {
				i += 1;
			}
			at_line_start = true;
			continue;
		}
		if c == b'\n' {
			at_line_start = true;
		} else {
			at_line_start = false;
		}
		out.push(c as char);
		i += 1;
	}
	out
}

/// Slice the body to the bytes that live inside the `[advisories]`
/// table, i.e. from the line that starts with `[advisories]` up to the
/// next line that begins with `[` (a new table header). Returns the
/// raw bytes so the caller can keep the same byte offsets throughout.
/// An empty slice means the section was not found.
fn advisories_section(body: &str) -> &[u8] {
	let bytes = body.as_bytes();
	let header = b"[advisories]";
	let header_len = header.len();
	let mut start: Option<usize> = None;
	let mut end: Option<usize> = None;
	let mut i = 0;
	let mut at_line_start = true;
	while i < bytes.len() {
		let c = bytes[i];
		if at_line_start
			&& c == b'['
			&& i + header_len <= bytes.len()
			&& &bytes[i..i + header_len] == header
		{
			if start.is_none() {
				start = Some(i);
				i += header_len;
				at_line_start = false;
				continue;
			} else {
				end = Some(i);
				break;
			}
		}
		if c == b'\n' {
			at_line_start = true;
		} else if c != b' ' && c != b'\t' && c != b'\r' {
			at_line_start = false;
		}
		i += 1;
	}
	let start = match start {
		Some(s) => s,
		None => return &[],
	};
	let end = end.unwrap_or(bytes.len());
	&bytes[start..end]
}

/// Find the `ignore = [ ... ]` array inside the `[advisories]` section
/// and return the bytes inside the brackets. Returns an empty slice if
/// the `ignore = [` opener is never seen. Nested `{ ... }` tables inside
/// the array are opened and closed correctly; quoted strings are opaque
/// to the bracket counter, so a `[` or `{` inside a string is not
/// counted. The closing `]` of `ignore = [...]` is the first one that
/// brings the bracket depth back to zero after the opener.
fn ignore_array(body: &[u8]) -> Vec<u8> {
	let mut open_at: Option<usize> = None;
	let mut i = 0;
	while i + 11 < body.len() {
		// Look for `ignore = [` at any column. Anything after a
		// `\n` is fair game; the section has been narrowed to the
		// [advisories] table already, so we won't pick up a stray
		// `ignore` somewhere else.
		if &body[i..i + 6] == b"ignore" {
			let mut j = i + 6;
			while j < body.len() && (body[j] == b' ' || body[j] == b'\t') {
				j += 1;
			}
			if j + 2 < body.len() && body[j] == b'=' && body[j + 1] == b' ' && body[j + 2] == b'[' {
				open_at = Some(j + 3);
				break;
			}
		}
		i += 1;
	}
	let open_at = match open_at {
		Some(o) => o,
		None => return Vec::new(),
	};
	let mut depth: i32 = 0;
	let mut k = open_at;
	while k < body.len() {
		let c = body[k];
		if c == b'"' {
			k += 1;
			while k < body.len() && body[k] != b'"' {
				k += 1;
			}
			if k < body.len() {
				k += 1;
			}
			continue;
		}
		if c == b'[' || c == b'{' {
			depth += 1;
			k += 1;
			continue;
		}
		if c == b']' {
			if depth == 0 {
				return body[open_at..k].to_vec();
			}
			depth -= 1;
			k += 1;
			continue;
		}
		if c == b'}' {
			if depth > 0 {
				depth -= 1;
			}
			k += 1;
			continue;
		}
		k += 1;
	}
	Vec::new()
}

/// Collect every `"X"` inside `bytes` whose whole value matches
/// `^RUSTSEC-[0-9]{4}-[0-9]{4}$`. The whole-value test is what stops an
/// id embedded in a longer string (e.g. `reason = "see RUSTSEC-..."`)
/// from being read as if it were a list entry.
fn extract_ids(bytes: &[u8]) -> Vec<String> {
	let mut out = Vec::new();
	let mut i = 0;
	while i < bytes.len() {
		let c = bytes[i];
		if c == b'"' {
			let start = i + 1;
			let mut j = start;
			while j < bytes.len() && bytes[j] != b'"' {
				j += 1;
			}
			if j >= bytes.len() {
				return out;
			}
			let token = &bytes[start..j];
			if is_rustsec_id(token) {
				out.push(std::str::from_utf8(token).unwrap_or("").to_string());
			}
			i = j + 1;
			continue;
		}
		i += 1;
	}
	out
}

fn is_rustsec_id(token: &[u8]) -> bool {
	let prefix = b"RUSTSEC-";
	if token.len() != 17 || !token.starts_with(prefix) {
		return false;
	}
	let bytes = &token[8..];
	if bytes[4] != b'-' {
		return false;
	}
	bytes[..4].iter().all(|b| b.is_ascii_digit()) && bytes[5..].iter().all(|b| b.is_ascii_digit())
}

/// One parser for both deny.toml and `.cargo/audit.toml`. Strips
/// comments, narrows to the `[advisories]` section, finds the
/// `ignore = [...]` array, and collects every quoted RUSTSEC id whose
/// value is exactly `RUSTSEC-YYYY-NNNN`.
fn ignored_ids(body: &str) -> Vec<String> {
	let stripped = strip_comments(body);
	let section = advisories_section(&stripped);
	let array = ignore_array(section);
	extract_ids(&array)
}

#[test]
fn the_two_ignore_lists_agree_on_advisory_ids() {
	let deny_ids: Vec<String> = ignored_ids(&read_repo("deny.toml"));
	let audit_ids: Vec<String> = ignored_ids(&read_repo(".cargo/audit.toml"));
	// Order does not matter: the contract is the SET of ids each tool
	// accepts. `BTreeSet` also makes the failure message stable.
	let deny_set: std::collections::BTreeSet<_> = deny_ids.iter().collect();
	let audit_set: std::collections::BTreeSet<_> = audit_ids.iter().collect();
	assert_eq!(
		deny_set, audit_set,
		"deny.toml and .cargo/audit.toml must list the same set of ignored \
		 advisories. deny.toml: {:?}, .cargo/audit.toml: {:?}. The release \
		 gate (release.yml) runs cargo-audit, which reads .cargo/audit.toml \
		 only; a difference here is a difference in what each tool approves.",
		deny_ids, audit_ids
	);
}

#[test]
fn the_parser_reads_what_it_claims() {
	// (a) Table form and plain-string form of the same id give the
	// same set, in either file. cargo-deny accepts both forms in the
	// same `[advisories].ignore` array.
	let deny_body = "\
[advisories]
ignore = [
    { id = \"RUSTSEC-0000-0000\", reason = \"x\" },
]
";
	let plain_body = "\
[advisories]
ignore = [\"RUSTSEC-0000-0000\"]
";
	let deny_set: std::collections::BTreeSet<_> = ignored_ids(deny_body).into_iter().collect();
	let plain_set: std::collections::BTreeSet<_> = ignored_ids(plain_body).into_iter().collect();
	assert_eq!(
		deny_set, plain_set,
		"table form and plain-string form of the same id must agree"
	);
	assert_eq!(
		deny_set,
		["RUSTSEC-0000-0000".to_string()].into_iter().collect()
	);

	// (b) A commented-out ignore line does not contribute to the set.
	let commented = "\
[advisories]
# ignore = [{ id = \"RUSTSEC-9999-9999\", reason = \"comment only\" }]
ignore = [{ id = \"RUSTSEC-0000-0000\", reason = \"real\" }]
";
	let ids: std::collections::BTreeSet<_> = ignored_ids(commented).into_iter().collect();
	assert_eq!(
		ids,
		["RUSTSEC-0000-0000".to_string()].into_iter().collect(),
		"commented-out ignore line must not contribute to the set, got {ids:?}"
	);

	// (c) A `reason = "..."` string that mentions another RUSTSEC id
	// does not add it: the whole-value test rejects it.
	let reason_mentions = "\
[advisories]
ignore = [
    { id = \"RUSTSEC-0000-0000\", reason = \"see RUSTSEC-1111-2222\" },
]
";
	let ids: std::collections::BTreeSet<_> = ignored_ids(reason_mentions).into_iter().collect();
	assert_eq!(
		ids,
		["RUSTSEC-0000-0000".to_string()].into_iter().collect(),
		"a reason string mentioning another RUSTSEC id must not add that \
		 id to the set, got {ids:?}"
	);

	// (d) Negative fixture: a deliberate mismatch between two
	// fixtures makes the parser pair disagree. A parser that
	// hardcoded the canonical set would still pass every assertion
	// above against any well-formed input; this fixture proves the
	// parsers actually read the bytes.
	let a = "\
[advisories]
ignore = [{ id = \"RUSTSEC-9999-9999\", reason = \"x\" }]
";
	let b = "\
[advisories]
ignore = [\"RUSTSEC-0000-0000\"]
";
	let sa: std::collections::BTreeSet<_> = ignored_ids(a).into_iter().collect();
	let sb: std::collections::BTreeSet<_> = ignored_ids(b).into_iter().collect();
	assert_ne!(
		sa, sb,
		"the negative fixture did not disagree between parses; the \
		 agreement test would have passed against drifted lists, \
		 which is the failure mode this case is here to prove the \
		 parser can catch. deny: {sa:?}, audit: {sb:?}"
	);
}
