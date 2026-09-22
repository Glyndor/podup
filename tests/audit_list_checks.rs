//! End-to-end checks for `podup audit --list-checks`.
//!
//! The drift contract an integrator relies on: the listing's ids are the
//! same strings a finding can emit (the audit registry is the single
//! source), and the listing answers without a compose file. Reading the
//! listing ids from the JSON output rather than from a hardcoded list
//! keeps the test in lockstep with `internal/audit/checks::CHECK_REGISTRY`
//! without duplicating either vocabulary here.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Run `podup` with `args` after `cd`-ing into `dir`. The working directory
/// has no compose file, so the listing is the only command in this file
/// that can succeed here. Same env-var scrub list as
/// `tests/audit_exit_codes.rs` so an inherited `PODUP_*` or `COMPOSE_*`
/// cannot masquerade as a named file path the binary would otherwise
/// resolve.
fn run_in(dir: &Path, args: &[&str]) -> Output {
	let mut cmd = Command::new(bin());
	cmd.args(args);
	cmd.current_dir(dir);
	for key in [
		"PODUP_LIBPOD_POOL",
		"PODUP_LIBCOD_POOL",
		"PODMAN_SOCKET",
		"DOCKER_HOST",
		"COMPOSE_PROJECT_NAME",
		"COMPOSE_PROFILES",
		"COMPOSE_FILE",
		"NO_COLOR",
	] {
		cmd.env_remove(key);
	}
	cmd.output().expect("run podup")
}

/// Render `body` to a tempfile alongside the listing run. Returns the path
/// the test passes to `-f`. The atomic counter is shared with `empty_dir`
/// so two tests running in parallel pick distinct names.
fn write_compose(body: &str) -> std::path::PathBuf {
	static COUNTER: AtomicU64 = AtomicU64::new(0);
	let n = COUNTER.fetch_add(1, Ordering::Relaxed);
	let path =
		std::env::temp_dir().join(format!("podup-list-checks-{}-{n}.yaml", std::process::id()));
	let _ = fs::remove_file(&path);
	fs::write(&path, body).unwrap();
	path
}

/// Build a tempdir with no compose file in it. Returns the directory so
/// `run_in` can `cd` there. The counter is shared with `write_compose` so
/// every call gets a unique name.
fn empty_dir() -> std::path::PathBuf {
	static COUNTER: AtomicU64 = AtomicU64::new(0);
	let n = COUNTER.fetch_add(1, Ordering::Relaxed);
	let dir =
		std::env::temp_dir().join(format!("podup-list-checks-dir-{}-{n}", std::process::id()));
	let _ = fs::remove_dir_all(&dir);
	fs::create_dir_all(&dir).unwrap();
	dir
}

#[test]
fn audit_list_checks_table_works_in_an_empty_directory() {
	// Drop into a tempdir that has no compose file. Without `--list-checks`
	// this would error out with `compose file not found`; with it the binary
	// answers the question an integrator asked and exits 0.
	let dir = empty_dir();
	let out = run_in(&dir, &["audit", "--list-checks"]);
	assert_eq!(
		out.status.code(),
		Some(0),
		"`--list-checks` must exit 0 in an empty directory; got {:?}\nstderr: {}\nstdout: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	// An always-on check is `<id>\t<description>`, exactly the two columns
	// 5.9.6 printed, so a reader that takes the description with `cut -f2`
	// keeps working. An opt-in check appends `\topt-in: <flag>` as a third
	// column. A regression that emitted JSON, or inserted a column in the
	// middle, fails here.
	let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
	assert!(
		!lines.is_empty(),
		"`--list-checks` printed no lines:\n{stdout}"
	);
	for line in &lines {
		let cols: Vec<&str> = line.split('\t').collect();
		let opt_in_row = cols.len() == 3 && cols[2].starts_with("opt-in: --");
		assert!(
			cols.len() == 2 || opt_in_row,
			"a row is `id<TAB>description`, or that plus `<TAB>opt-in: --flag`: {line:?}"
		);
		assert!(
			!cols[0].is_empty() && !cols[1].is_empty(),
			"empty id or description: {line:?}"
		);
		assert!(
			!cols[1].starts_with("--") && cols[1] != "-",
			"the second column is the description, not an opt-in marker: {line:?}"
		);
	}
	// Spot-check four known ids; if the registry dropped `secret_in_environment`
	// or `no_pids_limit` between releases the integrator's diff would name it,
	// and the test catches a regression where the listing drops one.
	for needed in [
		"secret_in_environment",
		"no_pids_limit",
		"unpinned_image",
		"host_namespace",
	] {
		let hit = lines
			.iter()
			.any(|l| l.split('\t').next().is_some_and(|id| id == needed));
		assert!(hit, "`--list-checks` listing missing `{needed}`:\n{stdout}");
	}
	// The wildcard check names its enabling flag in the trailing column.
	let wildcard_line = lines
		.iter()
		.find(|l| l.starts_with("port_published_on_wildcard\t"))
		.unwrap_or_else(|| panic!("wildcard row missing:\n{stdout}"));
	assert!(
		wildcard_line.ends_with("\topt-in: --wildcard-binds"),
		"the wildcard row must end with its opt-in flag: {wildcard_line:?}"
	);
	// Machine output never carries escapes, even when TTY detection would not
	// have enabled them. `--list-checks` builds its lines from `&'static str`
	// ids only, so this should hold; the assertion guards any future change
	// that interpolated user input into the listing.
	assert!(
		!stdout.contains('\u{1b}'),
		"`--list-checks` listing must not carry escapes: {stdout:?}"
	);
}

#[test]
fn audit_list_checks_json_emits_object_per_entry_in_a_directory_with_no_compose() {
	// Same no-`-f`, no-compose-file precondition. The JSON shape mirrors
	// `audit --format json` (`{"checks":[...]}` instead of `{"findings":[...]}`),
	// so a consumer reading either on the same wire sees the same envelope.
	let dir = empty_dir();
	let out = run_in(&dir, &["audit", "--list-checks", "--format", "json"]);
	assert_eq!(
		out.status.code(),
		Some(0),
		"`--list-checks --format json` must exit 0; got {:?}\nstderr: {}\nstdout: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		!stdout.contains('\u{1b}'),
		"JSON listing must not carry escapes: {stdout:?}"
	);
	let v: serde_json::Value = serde_json::from_str(&stdout).expect("JSON parses");
	let arr = v
		.get("checks")
		.and_then(|c| c.as_array())
		.expect("`checks` array");
	assert!(
		!arr.is_empty(),
		"`--list-checks --format json` produced an empty array: {stdout}"
	);
	let wanted_keys: std::collections::HashSet<&str> =
		["description", "id", "opt_in"].into_iter().collect();
	for entry in arr {
		let keys: std::collections::HashSet<&str> = entry
			.as_object()
			.expect("object")
			.keys()
			.map(|k| k.as_str())
			.collect();
		assert_eq!(keys, wanted_keys, "entry keys drifted: {entry:?}");
		let id = entry.get("id").and_then(|s| s.as_str()).unwrap_or_default();
		let desc = entry
			.get("description")
			.and_then(|s| s.as_str())
			.unwrap_or_default();
		assert!(
			!id.is_empty() && !desc.is_empty(),
			"empty id or description: {entry:?}"
		);
	}
	// The wildcard check is the only opt-in check the registry carries
	// today; its `opt_in` field must name `--wildcard-binds` exactly
	// (`#1881`). Every other entry must have `null`.
	let wildcard_entry = arr
		.iter()
		.find(|e| e.get("id").and_then(|s| s.as_str()) == Some("port_published_on_wildcard"))
		.expect("wildcard entry must appear in the listing");
	let opt_in = wildcard_entry
		.get("opt_in")
		.and_then(|v| v.as_str())
		.unwrap_or_else(|| panic!("wildcard `opt_in` must be a string: {wildcard_entry:?}"));
	assert_eq!(opt_in, "--wildcard-binds");
	for entry in arr {
		let id = entry.get("id").and_then(|s| s.as_str()).unwrap_or_default();
		if id == "port_published_on_wildcard" {
			continue;
		}
		let opt_in = entry
			.get("opt_in")
			.unwrap_or_else(|| panic!("non-wildcard `{id}` must carry `opt_in`: {entry:?}"));
		assert!(
			opt_in.is_null(),
			"non-wildcard `{id}` must have `opt_in: null`; got: {opt_in:?}"
		);
	}
}

#[test]
fn audit_list_checks_conflicts_with_strict_at_parse_time() {
	// `--strict` only flips the exit code on a run that produces findings;
	// `--list-checks` produces no findings, so silently accepting `--strict`
	// here is the silent-flag-drop #1840 warned about. The combination is
	// rejected with `exit 2` and a clap `error:` line.
	let dir = empty_dir();
	let out = run_in(&dir, &["audit", "--list-checks", "--strict"]);
	assert_eq!(
		out.status.code(),
		Some(2),
		"`--list-checks --strict` must be a clap usage error; got {:?}\nstderr: {}\nstdout: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
	);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains("error:"),
		"`--list-checks --strict` must print clap's error banner:\n{stderr}"
	);
	// And the inverse: `--strict` alone (no `--list-checks`) still parses
	// cleanly here; whether it then reaches the audit path or fails for lack
	// of a compose file is the audit branch's concern, not the parse. We only
	// care that the parse does not refuse the standalone form.
	let out_strict_only = run_in(&dir, &["audit", "--strict"]);
	assert_ne!(
		out_strict_only.status.code(),
		Some(2),
		"`audit --strict` alone must parse; got:\nstderr: {}\nstdout: {}",
		String::from_utf8_lossy(&out_strict_only.stderr),
		String::from_utf8_lossy(&out_strict_only.stdout),
	);
}

/// The drift contract: every id the listing prints is one a finding can
/// emit, and every id a finding can emit is one the listing prints. The test
/// does not hardcode any id; it reads the listing's ids from the JSON, then
/// runs a compose file that fires every check and reads the findings' ids
/// from the audit JSON, and asserts the two sets agree.
///
/// The findings run enables every opt-in flag the registry declares
/// (`--wildcard-binds` today). A future opt-in check whose registry
/// `opt_in` is set but whose flag never reaches the CLI would
/// otherwise silently drop out of the dirty fixture: the listing
/// would still name the id, but the findings run would never emit
/// it, and the two sets would diverge with no test to catch it
/// (`#1881`).
#[test]
fn audit_list_checks_ids_match_the_ids_a_finding_can_emit() {
	// 1. Listing side.
	let dir = empty_dir();
	let listing = run_in(&dir, &["audit", "--list-checks", "--format", "json"]);
	assert!(listing.status.success(), "listing run failed");
	let listing_json: serde_json::Value =
		serde_json::from_str(&String::from_utf8_lossy(&listing.stdout))
			.expect("listing parses as JSON");
	let listing_ids: std::collections::BTreeSet<String> = listing_json
		.get("checks")
		.and_then(|c| c.as_array())
		.expect("`checks` array")
		.iter()
		.map(|e| {
			e.get("id")
				.and_then(|s| s.as_str())
				.unwrap_or("")
				.to_string()
		})
		.filter(|s| !s.is_empty())
		.collect();
	assert!(
		!listing_ids.is_empty(),
		"listing produced no ids; the registry is empty"
	);

	// 2. Findings side: one compose file that simultaneously fails every
	// check the registry carries. The same shape the in-process drift guard
	// in `internal/audit/audit_tests.rs` uses; the duplication is intentional
	// because this test runs against the compiled binary and asserts the
	// contract end-to-end rather than against `CHECK_REGISTRY` directly.
	//
	// `0.0.0.0:6379:6379` is the wildcard-check input; without it the
	// opt-in check would never fire on this fixture, and the drift
	// contract would not notice an opt-in check whose id drifted off
	// the registry.
	let body = "\
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
	let path = write_compose(body);
	let p = path.to_str().unwrap();
	let findings = Command::new(bin())
		.args(["-f", p, "audit", "--format", "json", "--wildcard-binds"])
		.env_remove("COMPOSE_FILE")
		.env_remove("NO_COLOR")
		.output()
		.expect("audit run");
	assert!(findings.status.success(), "findings run failed");
	let findings_json: serde_json::Value =
		serde_json::from_str(&String::from_utf8_lossy(&findings.stdout))
			.expect("findings parses as JSON");
	let findings_ids: std::collections::BTreeSet<String> = findings_json
		.get("findings")
		.and_then(|c| c.as_array())
		.expect("`findings` array")
		.iter()
		.map(|e| {
			e.get("check")
				.and_then(|s| s.as_str())
				.unwrap_or("")
				.to_string()
		})
		.filter(|s| !s.is_empty())
		.collect();
	let _ = fs::remove_file(&path);

	// 3. The two sets must match. A drift on either side would name a check
	// the audit could not deliver, or hide one an integrator's diff expects
	// to see. Keep them in lockstep.
	assert_eq!(
		findings_ids, listing_ids,
		"every id the listing names must be one a finding can emit, and vice versa.\n\
		 listing: {:?}\nfindings: {:?}",
		listing_ids, findings_ids,
	);
}

// `--wildcard-binds` only affects an audit run, so combining it with
// `--list-checks` is refused at parse time rather than accepted and
// ignored, the same as `--strict`.
#[test]
fn audit_list_checks_refuses_wildcard_binds() {
	let dir = tempfile::tempdir().expect("tempdir");
	let out = std::process::Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["audit", "--list-checks", "--wildcard-binds"])
		.current_dir(dir.path())
		.output()
		.expect("run podup");
	assert!(
		!out.status.success(),
		"`--list-checks --wildcard-binds` must be refused; stdout: {}",
		String::from_utf8_lossy(&out.stdout)
	);
}
