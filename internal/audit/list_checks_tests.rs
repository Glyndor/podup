//! `audit --list-checks` (table and JSON), the registry-to-findings drift
//! guard, and that every id in the JSON listing is also a `Finding.check`
//! the audit path can produce.
//!
//! `audit --list-checks` is the operator-facing drift detector: an integrator
//! diffs the listing between releases to learn which checks the binary now
//! carries. The contract ("the listing prints ids a finding can emit, and
//! vice versa") reads from a single source: `CHECK_REGISTRY`. These tests
//! pin it without coupling to the listing renderer's bytes.

use podup::parse_str;

#[allow(unused_imports)]
use super::checks::CHECK_REGISTRY;
use super::{audit_file, render_list_checks_json_to, render_list_checks_table_to};

/// One compose file that simultaneously fails every check the registry
/// carries. Each check fires on its known-bad input; the union of
/// `Finding.check` ids produced by the audit must equal the registry's ids.
/// If either side drifts (hand-edited id in `finding(name, "...", ...)` that
/// the registry does not mirror, or a registry entry whose `run` function
/// emits a different id), this test names the gap.
const DIRTY_COMPOSE: &str = "\
services:
  web:
    image: alpine
    privileged: true
    pid: host
    cap_add: [SYS_ADMIN]
    read_only: false
    cap_drop: []
    security_opt: []
    pids_limit: null
    mem_limit: not-a-size
    userns_mode: null
    environment:
      - DB_PASSWORD=hunter2
";

#[test]
fn check_registry_contains_an_entry_for_every_finding_id_the_audit_emits() {
	// Direction 1: every id the audit emits is named by the registry. If a
	// check function changes its hardcoded id without updating the registry,
	// the listing would name a different id and `audit --list-checks` would
	// claim a check that never fires; the integrator's drift detector
	// catches it. If a registry entry points at a function that no longer
	// emits its id, the reverse, the listing would under-report.
	let file = parse_str(DIRTY_COMPOSE).expect("dirty compose parses");
	let report = audit_file(&file);
	let mut found: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
	for f in &report.findings {
		assert!(
			CHECK_REGISTRY.iter().any(|c| c.id == f.check),
			"the audit emitted `{}` but the registry does not name it; \
			 either the check function's hardcoded id drifted or a new \
			 check was added without an entry in CHECK_REGISTRY",
			f.check
		);
		found.insert(f.check);
	}
	let wanted: std::collections::BTreeSet<&'static str> =
		CHECK_REGISTRY.iter().map(|c| c.id).collect();
	assert_eq!(
		found, wanted,
		"every check the audit carries must fire on DIRTY_COMPOSE so the \
		 drift detector cannot miss a registry entry whose run no longer fires"
	);
}

/// `render_list_checks_table_to` writes one `<id>\t<description>` line per
/// registry entry, in registry order. The id is the `&'static str` the
/// run function emits; the description is the prose the listing prints.
#[test]
fn list_checks_table_renders_one_id_and_description_per_registry_entry() {
	let mut buf: Vec<u8> = Vec::new();
	render_list_checks_table_to(&mut buf).expect("render");
	let out = String::from_utf8_lossy(&buf);
	let lines: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
	assert_eq!(
		lines.len(),
		CHECK_REGISTRY.len(),
		"one line per registry entry:\n{out}"
	);
	for (line, entry) in lines.iter().zip(CHECK_REGISTRY.iter()) {
		let (id, rest) = line
			.split_once('\t')
			.unwrap_or_else(|| panic!("missing tab between id and description: {line:?}"));
		assert_eq!(id, entry.id, "id is the registry's id");
		assert_eq!(
			rest, entry.description,
			"description is the registry's description"
		);
	}
}

/// `render_list_checks_json_to` returns a `{"checks":[...]}` envelope with
/// one `{id, description}` object per registry entry, in registry order.
/// The keys are stable (`description` before `id` alphabetically) so a CI
/// diff between releases sees byte-exact lines on unchanged entries.
#[test]
fn list_checks_json_emits_object_per_entry_with_stable_keys() {
	let mut buf: Vec<u8> = Vec::new();
	render_list_checks_json_to(&mut buf).expect("render");
	let stdout = String::from_utf8_lossy(&buf);
	assert!(
		!stdout.contains('\u{1b}'),
		"JSON listing must not carry escapes: {stdout:?}"
	);
	let v: serde_json::Value = serde_json::from_str(&stdout).expect("JSON parses");
	let arr = v
		.get("checks")
		.and_then(|c| c.as_array())
		.expect("`checks` array");
	assert_eq!(
		arr.len(),
		CHECK_REGISTRY.len(),
		"one object per registry entry: {arr:?}"
	);
	for (entry, registry) in arr.iter().zip(CHECK_REGISTRY.iter()) {
		let id = entry
			.get("id")
			.and_then(|s| s.as_str())
			.unwrap_or_else(|| panic!("missing `id` field: {entry:?}"));
		let desc = entry
			.get("description")
			.and_then(|s| s.as_str())
			.unwrap_or_else(|| panic!("missing `description` field: {entry:?}"));
		assert_eq!(id, registry.id);
		assert_eq!(desc, registry.description);
	}
	// Every object must carry the same shape: never the registry-id list
	// reordered, never a field dropped. The drift detector reads both
	// halves of every entry.
	let wanted_keys: std::collections::BTreeSet<&'static str> =
		["description", "id"].into_iter().collect();
	for entry in arr {
		let keys: std::collections::BTreeSet<&str> = entry
			.as_object()
			.expect("object")
			.keys()
			.map(|k| k.as_str())
			.collect();
		assert_eq!(keys, wanted_keys, "entry keys drifted: {entry:?}");
	}
}
