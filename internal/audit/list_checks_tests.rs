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

/// Every opt-in flag the registry declares. Building this list by
/// scanning the registry keeps the drift test honest: a future
/// opt-in check whose `opt_in` field is added without a matching
/// slice entry would silently slip past the fixture, and the new
/// check would never appear in either the registry's id set or the
/// finding set. Hand-edited here on purpose: the in-binary drift
/// test is the only gate that catches a registry whose opt-in marker
/// is set but whose flag never reaches `audit_file` (`#1881`).
const ALL_OPT_IN: &[&str] = &["--wildcard-binds"];

/// One compose file that simultaneously fails every check the registry
/// carries. Each check fires on its known-bad input; the union of
/// `Finding.check` ids produced by the audit must equal the registry's ids.
/// If either side drifts (hand-edited id in `finding(name, "...", ...)` that
/// the registry does not mirror, or a registry entry whose `run` function
/// emits a different id), this test names the gap.
///
/// The fixture carries both a `5432:5432` (the all-interfaces check) and
/// a `0.0.0.0:5432:5432` (the wildcard check) so a regression that
/// either removed or merged the two checks is caught the same way
/// (`#1881`). Every opt-in flag is enabled when the drift test runs,
/// so a check whose id drifted off the registry still fires here.
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
    ports:
      - \"5432:5432\"
      - \"0.0.0.0:6379:6379\"
    volumes:
      - /run/user/1000/podman/podman.sock:/sock
";

#[test]
fn check_registry_contains_an_entry_for_every_finding_id_the_audit_emits() {
	// Direction 1: every id the audit emits is named by the registry. If a
	// check function changes its hardcoded id without updating the registry,
	// the listing would name a different id and `audit --list-checks` would
	// claim a check that never fires; the integrator's drift detector
	// catches it. If a registry entry points at a function that no longer
	// emits its id, the reverse, the listing would under-report.
	//
	// `ALL_OPT_IN` is passed so an opt-in check whose id drifted off the
	// registry still fires here: the test would otherwise see a registry
	// entry whose run function never produced the id, and the missing-fire
	// half of the drift contract would slip past (`#1881`).
	let file = parse_str(DIRTY_COMPOSE).expect("dirty compose parses");
	let report = audit_file(&file, ALL_OPT_IN);
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

/// An opt-in check that is not enabled on this run must not appear in
/// the findings list, even when its bad-shape input is present in the
/// compose file. The wildcard check is the only opt-in check the
/// registry carries today; `&[]` says the CLI has not enabled it, so
/// `port_published_on_wildcard` must be silent on the same
/// `0.0.0.0:6379:6379` mapping the drift test fires (`#1881`).
#[test]
fn opt_in_check_does_not_fire_when_its_flag_is_absent() {
	let file = parse_str(DIRTY_COMPOSE).expect("dirty compose parses");
	let report = audit_file(&file, &[]);
	let wildcard: Vec<&str> = report
		.findings
		.iter()
		.filter(|f| f.check == "port_published_on_wildcard")
		.map(|f| f.reason.as_str())
		.collect();
	assert!(
		wildcard.is_empty(),
		"`port_published_on_wildcard` must not fire without --wildcard-binds: \
		 {wildcard:#?}"
	);
}

/// Same compose, opt-in enabled: the wildcard finding now fires for the
/// `0.0.0.0:6379:6379` mapping, and the all-interfaces finding still
/// fires for the `5432:5432` mapping. The two checks are disjoint on
/// the same compose file, so a regression that merged them would
/// either drop one finding or double-count the other (`#1881`).
#[test]
fn opt_in_check_fires_when_its_flag_is_present() {
	let file = parse_str(DIRTY_COMPOSE).expect("dirty compose parses");
	let report = audit_file(&file, ALL_OPT_IN);
	let ids: std::collections::BTreeSet<&str> = report.findings.iter().map(|f| f.check).collect();
	assert!(
		ids.contains("port_published_on_wildcard"),
		"`port_published_on_wildcard` must fire with --wildcard-binds: {ids:?}"
	);
	assert!(
		ids.contains("port_published_on_all_interfaces"),
		"`port_published_on_all_interfaces` must still fire on 5432:5432: {ids:?}"
	);
	// And the all-interfaces finding must NOT carry the wildcard id:
	// a merge regression that emitted both ids on the same mapping
	// would put `port_published_on_wildcard` next to `5432` (not
	// `0.0.0.0`), and the test would still pass on the wildcard id
	// being present. Pin the reason labels to the port string they
	// cover so a merge regression is caught.
	let wildcard_reasons: Vec<&str> = report
		.findings
		.iter()
		.filter(|f| f.check == "port_published_on_wildcard")
		.map(|f| f.reason.as_str())
		.collect();
	let all_interfaces_reasons: Vec<&str> = report
		.findings
		.iter()
		.filter(|f| f.check == "port_published_on_all_interfaces")
		.map(|f| f.reason.as_str())
		.collect();
	assert!(
		wildcard_reasons.iter().any(|r| r.contains("6379")),
		"wildcard finding must name the 6379 mapping: {wildcard_reasons:?}"
	);
	assert!(
		all_interfaces_reasons.iter().any(|r| r.contains("5432")),
		"all-interfaces finding must name the 5432 mapping: {all_interfaces_reasons:?}"
	);
	assert!(
		!wildcard_reasons.iter().any(|r| r.contains("5432")),
		"wildcard finding must NOT cover the 5432 mapping: {wildcard_reasons:?}"
	);
}

/// `render_list_checks_table_to` writes one `<id>\t<opt_in>\t<description>`
/// line per registry entry, in registry order. The id is the `&'static
/// str` the run function emits; the description is the prose the
/// listing prints; the opt_in column carries the flag name for opt-in
/// checks and `-` for always-on checks (`#1881`).
#[test]
fn list_checks_table_renders_one_id_opt_in_and_description_per_registry_entry() {
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
		let expected = match entry.opt_in {
			None => format!("{}\t{}", entry.id, entry.description),
			Some(flag) => format!("{}\t{}\topt-in: {}", entry.id, entry.description, flag),
		};
		assert_eq!(
			*line, expected,
			"an always-on row is `id<TAB>description`; an opt-in row appends `<TAB>opt-in: <flag>`"
		);
	}
}

/// The opt-in column must name the registry's flag for the wildcard
/// check. A regression that flipped the registry to a different flag
/// (or omitted the column for this entry) would not flip the
/// per-entry shape assertion above, so the column-value assertion is
/// its own test (`#1881`).
#[test]
fn list_checks_table_opt_in_column_names_the_wildcard_flag() {
	let mut buf: Vec<u8> = Vec::new();
	render_list_checks_table_to(&mut buf).expect("render");
	let out = String::from_utf8_lossy(&buf);
	let wildcard_line = out
		.lines()
		.find(|l| l.starts_with("port_published_on_wildcard\t"))
		.unwrap_or_else(|| panic!("wildcard row missing from listing:\n{out}"));
	assert!(
		wildcard_line.ends_with("\topt-in: --wildcard-binds"),
		"the wildcard row ends with its enabling flag: {wildcard_line:?}"
	);
}

/// `render_list_checks_json_to` returns a `{"checks":[...]}` envelope with
/// one `{id, description, opt_in}` object per registry entry, in
/// registry order. The keys are stable (`description` before `id`
/// before `opt_in` alphabetically) so a CI diff between releases sees
/// byte-exact lines on unchanged entries.
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
		let opt_in = entry
			.get("opt_in")
			.unwrap_or_else(|| panic!("missing `opt_in` field on `{id}`: {entry:?}"));
		// The `opt_in` field carries the flag name as a JSON string for
		// opt-in checks and `null` for always-on checks. A consumer
		// reading it never needs to branch on "present vs absent";
		// the field is always there, only its value differs.
		assert_eq!(
			opt_in,
			&serde_json::Value::from(registry.opt_in),
			"`opt_in` field on `{id}` must mirror the registry: {entry:?}"
		);
	}
	// Every object must carry the same shape: never the registry-id list
	// reordered, never a field dropped. The drift detector reads both
	// halves of every entry.
	let wanted_keys: std::collections::BTreeSet<&'static str> =
		["description", "id", "opt_in"].into_iter().collect();
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
