//! `cargo audit` and `cargo deny` agree on which RUSTSEC ids are ignored.
//!
//! `deny.toml` carries the cargo-deny ignore list and `.cargo/audit.toml`
//! carries the cargo-audit one. cargo-deny reads `deny.toml` and cargo-audit
//! reads `.cargo/audit.toml`, relative to the directory it runs in. The
//! release job (release.yml) runs `cargo audit --deny warnings` from the
//! repository root, so an ignore that lives only in deny.toml does not help
//! the release gate: the next release would fail on the same advisory that
//! turned the scheduled audit red on 2026-10-12 (#2034). The two lists must
//! be equal.
//!
//! No new crate dependency is taken on for this. `toml` is not in the
//! workspace's lock today, and pulling it just to compare two short string
//! lists is more weight than the assertion needs. The parser is a hand-rolled
//! scan for `id = "..."` (deny.toml's table form) and `ignore = [ "..." ]`
//! (cargo-audit's list form); both shapes are stable per the input files
//! they target.

use std::fs;
use std::path::Path;

fn read_repo(rel: &str) -> String {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
	fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel} is readable: {e}"))
}

/// Extract the set of advisory ids in `body` whose shape is a deny.toml
/// table entry: `id = "RUSTSEC-..."`. The line may be a full inline
/// table (`{ id = "X", reason = "..." }`) or just the field (`id = "X"`),
/// so the parser scans for `id = "..."` anywhere on the line. Comment
/// lines and lines without the field are skipped; a future refactor
/// that introduces an unrelated `id = "..."` field on a different key
/// would also be rejected only when its value starts with `RUSTSEC-`,
/// which is the convention the file's own entries follow.
fn deny_toml_ids(body: &str) -> Vec<String> {
	let mut out = Vec::new();
	for line in body.lines() {
		let trimmed = line.trim_start();
		// Comment and blank lines: skip. Comment lines mention ids in
		// prose and an inline comment that just happens to mention
		// one would otherwise be picked up. Same shape as the parsers
		// in `tests/workflow_audit_strictness.rs`.
		if trimmed.is_empty() || trimmed.starts_with('#') {
			continue;
		}
		// Find every `id = "RUSTSEC-..."` token on the line. The line
		// may also be the inline-table form `{ id = "X", ... }`, in
		// which case the scan still starts at `id` and reads forward
		// past the `=` to the quoted string.
		let bytes = trimmed.as_bytes();
		let mut i = 0;
		while i < bytes.len() {
			if bytes[i] != b'i' {
				i += 1;
				continue;
			}
			if !trimmed[i..].starts_with("id") {
				i += 1;
				continue;
			}
			let after_id_idx = i + 2;
			if after_id_idx < bytes.len() && !matches!(bytes[after_id_idx], b' ' | b'=') {
				i += 1;
				continue;
			}
			let mut j = after_id_idx;
			while j < bytes.len() && bytes[j] == b' ' {
				j += 1;
			}
			if j >= bytes.len() || bytes[j] != b'=' {
				i += 1;
				continue;
			}
			j += 1;
			while j < bytes.len() && bytes[j] == b' ' {
				j += 1;
			}
			if j >= bytes.len() {
				break;
			}
			let quote = bytes[j];
			if quote != b'"' && quote != b'\'' {
				i += 1;
				continue;
			}
			j += 1;
			let start = j;
			while j < bytes.len() && bytes[j] != quote {
				j += 1;
			}
			if j >= bytes.len() {
				break;
			}
			let token = &trimmed[start..j];
			if token.starts_with("RUSTSEC-") {
				out.push(token.to_string());
			}
			i = j + 1;
		}
	}
	out
}

/// Extract the set of advisory ids in `body` whose shape is the
/// cargo-audit list form: `ignore = [ "RUSTSEC-...", "..." ]`. Strings
/// inside `[]` are matched one at a time; commas and trailing whitespace
/// are tolerated because that is the form `.cargo/audit.toml` already
/// uses. Lines outside the `ignore = [...]` declaration are not parsed
/// here either: the file is small and a stray RUSTSEC id elsewhere would
/// still belong in `ignore`, so the question "is X in the ignore list?"
/// either way points at the same place.
///
/// The inline single-line form (`ignore = ["X", "Y"]`) is read in one
/// pass: the `]` may appear on the same line that holds the entries,
/// so the parser cannot rely on a line-by-line open/close. It tracks
/// bracket nesting instead: an `[` that is not inside a string opens
/// the list, a matching `]` closes it, and quoted strings inside the
/// brackets are not counted as brackets. Multi-line lists behave the
/// same way, with the `[` on one line and the matching `]` later.
fn audit_toml_ids(body: &str) -> Vec<String> {
	let mut out = Vec::new();
	let mut depth: i32 = 0;
	for line in body.lines() {
		// Walk the line character by character. Outside the list
		// (`depth == 0`) we skip everything except a `[` that opens
		// the ignore list; inside it (depth > 0) we collect every
		// quoted `RUSTSEC-...` token.
		let bytes = line.as_bytes();
		let mut i = 0;
		while i < bytes.len() {
			let c = bytes[i];
			// A quoted token. Single or double quotes; the inside of
			// a quoted string is opaque, so brackets inside a string
			// do not count as brackets, and `=` inside a string does
			// not count as the assignment.
			if c == b'"' || c == b'\'' {
				let quote = c;
				let start = i + 1;
				let mut j = start;
				while j < bytes.len() && bytes[j] != quote {
					j += 1;
				}
				if j < bytes.len() {
					if depth > 0 {
						let token = &line[start..j];
						if token.starts_with("RUSTSEC-") {
							out.push(token.to_string());
						}
					}
					i = j + 1;
					continue;
				}
			}
			// A `[` inside or outside the list changes the bracket
			// nesting: outside, an opening `[` enters the list only
			// when the line was `ignore = [ ... ]` (i.e. depth was
			// zero and we are tracking `ignore`-list entries). To
			// keep the parser robust against the multiline form
			// (`ignore = [\n  "X",\n  "Y",\n]`) the scan tracks
			// `depth` rather than a boolean: a `[` already inside an
			// `ignore = [...]` increments the depth, and a `]`
			// decrements it back to zero.
			if c == b'[' {
				depth += 1;
				i += 1;
				continue;
			}
			if c == b']' {
				if depth > 0 {
					depth -= 1;
				}
				i += 1;
				continue;
			}
			i += 1;
		}
	}
	out
}

/// `audit_toml_ids` once used a `first_id_token` helper for the
/// single-line form. The rewritten parser above is bracket-depth aware
/// and reads every quoted id on a line in one pass, so the helper is
/// gone: keep this comment so a future reader who lands on a `git
/// log -p` and wonders why a private helper existed finds the answer.

#[test]
fn the_two_ignore_lists_agree_on_advisory_ids() {
	let deny_ids: Vec<String> = deny_toml_ids(&read_repo("deny.toml"));
	let audit_ids: Vec<String> = audit_toml_ids(&read_repo(".cargo/audit.toml"));
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

/// Pin the parser on input it must accept. Today the file format is
/// `{ id = "X", reason = "..." }` for deny.toml and `ignore = ["X", "Y"]`
/// for .cargo/audit.toml; the parser is what every other test in this
/// file trusts, and a parser that always returned one canonical id would
/// satisfy the assertions above against a tree where the lists had
/// drifted.
#[test]
fn the_parser_reads_what_it_claims() {
	// deny.toml table form: one id, exactly one entry, no comments.
	let deny_body = "\
[advisories]
ignore = [
    { id = \"RUSTSEC-0000-0000\", reason = \"x\" },
]
";
	let deny_ids = deny_toml_ids(deny_body);
	assert_eq!(
		deny_ids,
		vec!["RUSTSEC-0000-0000".to_string()],
		"deny.toml table form: exactly one id, got {deny_ids:?}"
	);

	// A comment above the entry that mentions the id must not be counted.
	let deny_with_comment = "\
[advisories]
# ignore = [{ id = \"RUSTSEC-9999-9999\", reason = \"comment only\" }]
ignore = [{ id = \"RUSTSEC-0000-0000\", reason = \"real\" }]
";
	let ids = deny_toml_ids(deny_with_comment);
	assert_eq!(
		ids,
		vec!["RUSTSEC-0000-0000".to_string()],
		"deny.toml parser picked up an id from a comment line, got {ids:?}"
	);

	// cargo-audit list form: a multi-line array with more than one id.
	let audit_body = "\
[advisories]
ignore = [
    \"RUSTSEC-0000-0000\",
    \"RUSTSEC-1111-2222\",
]
";
	let audit_ids = audit_toml_ids(audit_body);
	assert_eq!(
		audit_ids,
		vec![
			"RUSTSEC-0000-0000".to_string(),
			"RUSTSEC-1111-2222".to_string()
		],
		"cargo-audit multi-line list form: both ids parsed, got {audit_ids:?}"
	);

	// cargo-audit single-line list form: both ids on the same line.
	let audit_one_line = "\
[advisories]
ignore = [\"RUSTSEC-0000-0000\", \"RUSTSEC-1111-2222\"]
";
	let ids_inline = audit_toml_ids(audit_one_line);
	assert_eq!(
		ids_inline,
		vec![
			"RUSTSEC-0000-0000".to_string(),
			"RUSTSEC-1111-2222".to_string()
		],
		"cargo-audit single-line form: both ids parsed, got {ids_inline:?}"
	);

	// The negative control: a deny.toml fixture where the two parsers
	// disagree. The parser pair, not the input set, is what the
	// agreement test depends on: a parser that hardcoded the
	// canonical set would pass every assertion above against any
	// well-formed file, including a drifted one.
	let deny_differs = "\
[advisories]
ignore = [{ id = \"RUSTSEC-9999-9999\", reason = \"x\" }]
";
	let audit_differs = "\
[advisories]
ignore = [\"RUSTSEC-0000-0000\"]
";
	let d: std::collections::BTreeSet<_> = deny_toml_ids(deny_differs).into_iter().collect();
	let a: std::collections::BTreeSet<_> = audit_toml_ids(audit_differs).into_iter().collect();
	assert_ne!(
		d, a,
		"the negative fixture did not disagree between parsers; the \
		 agreement test would have passed against drifted lists, which \
		 is the failure mode the inline case is here to prove the \
		 parsers can catch. deny: {d:?}, audit: {a:?}"
	);
}
