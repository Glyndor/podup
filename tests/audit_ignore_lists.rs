//! `cargo audit` and `cargo deny` read two different files but must agree
//! on which RUSTSEC ids are ignored. `deny.toml` carries the cargo-deny
//! list and `.cargo/audit.toml` carries the cargo-audit one. The release
//! job (`release.yml`) runs `cargo audit --deny warnings` from the
//! repository root, so an ignore that lives only in deny.toml does not
//! help the release gate. The two lists must be equal. This test only
//! accepts one canonical layout per file and refuses the rest.
use std::fs;
use std::path::Path;
fn read_repo(rel: &str) -> String {
	fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
		.unwrap_or_else(|e| panic!("{rel} readable: {e}"))
}
fn is_rustsec(s: &str) -> Option<&str> {
	let b = s.as_bytes();
	let d = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
	(b.len() == 17 && b.starts_with(b"RUSTSEC-") && b[12] == b'-' && d(8..12) && d(13..17))
		.then_some(s)
}
const DENY_SHAPE: &str = "`{ id = \"RUSTSEC-YYYY-NNNN\", reason = \"...\" },`";
fn parse_entry(file: &str, l: &str) -> Option<String> {
	let s = l.strip_suffix(',')?.trim();
	if file.ends_with("/audit.toml") {
		is_rustsec(s.strip_prefix('"')?.strip_suffix('"')?).map(String::from)
	} else {
		let inner = s.strip_prefix('{')?.strip_suffix('}')?.trim();
		let (left, right) = inner.split_once(',')?;
		let id = left
			.trim()
			.strip_prefix("id = ")?
			.strip_prefix('"')?
			.strip_suffix('"')?;
		let rv = right
			.trim()
			.strip_prefix("reason = ")?
			.strip_prefix('"')?
			.strip_suffix('"')?;
		(!rv.contains('"')).then_some(())?;
		is_rustsec(id).map(String::from)
	}
}
fn ignored(file: &str, body: &str, shape: &str) -> Vec<String> {
	macro_rules! bad { ($l:expr) => { panic!("{file}: write the ignore list in the canonical form, one entry per line: {shape}; found: {}", $l) } }
	let dup = || panic!("{file}: expected exactly one ignore key");
	let mut in_adv = false;
	let mut in_arr = false;
	let mut seen = false;
	let mut out = Vec::new();
	for line in body.lines() {
		let l = line.trim();
		in_adv = in_adv || l == "[advisories]";
		if !in_adv {
			continue;
		}
		let ig = l.starts_with("ignore =") || l.starts_with("ignore=");
		match (in_arr, ig, l) {
			(false, _, l) if l.is_empty() || l.starts_with('#') => continue,
			(false, _, l) if l.starts_with('[') && l.ends_with(']') && l != "[advisories]" => break,
			(false, false, _) => continue,
			(false, true, l) => {
				if seen {
					dup();
				}
				seen = true;
				match l {
					"ignore = []" => return Vec::new(),
					"ignore = [" => in_arr = true,
					_ => bad!(l),
				}
			}
			(true, true, _) => dup(),
			(true, false, "]") => return out,
			(true, false, l) => match parse_entry(file, l) {
				Some(id) => out.push(id),
				None => bad!(l),
			},
		}
	}
	if !seen {
		dup();
	}
	panic!("{file}: missing closing `]` for ignore array");
}
fn cargo_ids(body: &str) -> Vec<String> {
	ignored(".cargo/audit.toml", body, "`\"RUSTSEC-YYYY-NNNN\",`")
}
fn deny_ids(body: &str) -> Vec<String> {
	ignored("deny.toml", body, DENY_SHAPE)
}
#[test]
fn the_two_ignore_lists_agree_on_advisory_ids() {
	let mut deny = deny_ids(&read_repo("deny.toml"));
	let mut audit = cargo_ids(&read_repo(".cargo/audit.toml"));
	deny.sort();
	audit.sort();
	assert_eq!(
		deny, audit,
		"deny.toml and .cargo/audit.toml disagree. deny: {deny:?}, audit: {audit:?}"
	);
}
#[test]
fn empty_ignore_array_yields_empty() {
	assert!(cargo_ids("[advisories]\nignore = []\n").is_empty());
}
#[test]
fn canonical_two_entries_read_in_order() {
	assert_eq!(
		cargo_ids(
			"[advisories]\nignore = [\n    \"RUSTSEC-2024-0001\",\n    \"RUSTSEC-2024-0002\",\n]\n"
		),
		vec![
			"RUSTSEC-2024-0001".to_string(),
			"RUSTSEC-2024-0002".to_string()
		]
	);
}
#[test]
#[should_panic(expected = "write the ignore list in the canonical form")]
fn commented_out_entry_panics() {
	let _ = cargo_ids(
		"[advisories]\nignore = [\n    # \"RUSTSEC-2024-0001\",\n    \"RUSTSEC-2024-0002\",\n]\n",
	);
}
#[test]
#[should_panic(expected = "write the ignore list in the canonical form")]
fn one_line_array_panics() {
	let _ = cargo_ids("[advisories]\nignore = [\"RUSTSEC-0000-0000\"]\n");
}
#[test]
#[should_panic(expected = "expected exactly one ignore key")]
fn second_ignore_key_panics() {
	let _ = cargo_ids("[advisories]\nignore = [\nignore = [],\n]\n");
}
#[test]
fn deny_entry_with_reason_mentioning_id_yields_only_id() {
	assert_eq!(
		deny_ids("[advisories]\nignore = [\n    { id = \"RUSTSEC-2024-0001\", reason = \"see RUSTSEC-9999-9999\" },\n]\n"),
		vec!["RUSTSEC-2024-0001".to_string()]
	);
}
