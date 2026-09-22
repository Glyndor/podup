//! Per-shape coverage for the `port_published_on_wildcard` check.
//!
//! Mirrors the structure of `checks_port_exposure_tests.rs`: every shape
//! that the `ports_published_on_wildcard` predicate in
//! `internal/compose/diagnostics/ignored_fields.rs` accepts or rejects
//! has a test here. The opt-in gate is exercised by `list_checks_tests
//! ::opt_in_check_does_not_fire_when_its_flag_is_absent` and the binary
//! test `tests/audit_exit_codes.rs`; this file owns the per-shape
//! `#1881` matrix itself.

use podup::parse_str;

use crate::audit::{audit_file, Finding};

/// Run every check against `yaml` with `--wildcard-binds` enabled and
/// return the flat list of findings. Every test in this file passes the
/// flag on, so the wildcard findings surface; the per-shape assertions
/// then filter by `check` to ignore the unrelated findings the same
/// compose file raises.
fn report_for(yaml: &str) -> Vec<Finding> {
	let file = parse_str(yaml).expect("compose parses");
	audit_file(&file, &["--wildcard-binds"]).findings
}

fn wildcard_reasons(findings: &[Finding]) -> Vec<&str> {
	findings
		.iter()
		.filter(|f| f.check == "port_published_on_wildcard")
		.map(|f| f.reason.as_str())
		.collect()
}

// ---------------------------------------------------------------------------
// short form
// ---------------------------------------------------------------------------

/// `0.0.0.0:5432:5432` is the explicit IPv4 wildcard. The check fires.
#[test]
fn audit_port_published_on_wildcard_flags_short_with_ipv4_wildcard() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "0.0.0.0:5432:5432"
"#;
	let findings = report_for(yaml);
	let reasons = wildcard_reasons(&findings);
	assert_eq!(
		reasons.len(),
		1,
		"exactly one finding expected: {findings:#?}"
	);
	assert!(
		reasons[0].contains("5432"),
		"reason must name the port; got: {}",
		reasons[0]
	);
	assert!(
		findings
			.iter()
			.any(|f| f.check == "port_published_on_wildcard" && f.service == "db"),
		"finding must attach to the `db` service: {findings:#?}"
	);
}

/// `[::]:5432:5432` is the IPv6 wildcard. The check fires.
#[test]
fn audit_port_published_on_wildcard_flags_short_with_ipv6_wildcard() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "[::]:5432:5432"
"#;
	let findings = report_for(yaml);
	let reasons = wildcard_reasons(&findings);
	assert_eq!(
		reasons.len(),
		1,
		"exactly one finding expected: {findings:#?}"
	);
	assert!(
		reasons[0].contains("5432"),
		"reason must name the port; got: {}",
		reasons[0]
	);
}

/// `127.0.0.1:5432:5432` is a specific loopback: a deliberate decision,
/// must NOT fire. The all-interfaces check stays silent too.
#[test]
fn audit_port_published_on_wildcard_passes_short_with_loopback_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "127.0.0.1:5432:5432"
"#;
	let findings = report_for(yaml);
	assert!(
		wildcard_reasons(&findings).is_empty(),
		"loopback must not fire: {findings:#?}"
	);
}

/// `192.168.1.10:5432:5432` is a private LAN bind: a deliberate
/// decision, must NOT fire.
#[test]
fn audit_port_published_on_wildcard_passes_short_with_private_lan_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "192.168.1.10:5432:5432"
"#;
	let findings = report_for(yaml);
	assert!(
		wildcard_reasons(&findings).is_empty(),
		"private LAN bind is a decision; must not fire: {findings:#?}"
	);
}

/// `[::1]:5432:5432` is a specific IPv6 loopback: must NOT fire.
#[test]
fn audit_port_published_on_wildcard_passes_short_with_ipv6_loopback() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "[::1]:5432:5432"
"#;
	let findings = report_for(yaml);
	assert!(
		wildcard_reasons(&findings).is_empty(),
		"IPv6 loopback must not fire: {findings:#?}"
	);
}

/// `5432:5432` (no host IP) is the all-interfaces check's job. The
/// wildcard check stays silent here: a regression that fired both
/// findings on the same mapping would double-count the same risk,
/// and the reason text would have to say two different things for
/// the same port.
#[test]
fn audit_port_published_on_wildcard_passes_short_without_host_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "5432:5432"
"#;
	let findings = report_for(yaml);
	assert!(
		wildcard_reasons(&findings).is_empty(),
		"no-IP short form is the all-interfaces check's job; wildcard must not \
		 fire: {findings:#?}"
	);
}

/// Container-only short form (`"5432"` with 0 colons) is not a publish,
/// neither check fires.
#[test]
fn audit_port_published_on_wildcard_passes_short_container_only() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "5432"
"#;
	let findings = report_for(yaml);
	assert!(
		wildcard_reasons(&findings).is_empty(),
		"container-only must not fire: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// long form
// ---------------------------------------------------------------------------

/// `host_ip: "0.0.0.0"` in the long form: explicit wildcard, fires.
#[test]
fn audit_port_published_on_wildcard_flags_long_with_ipv4_wildcard() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - target: 5432
        published: 5432
        host_ip: 0.0.0.0
"#;
	let findings = report_for(yaml);
	let reasons = wildcard_reasons(&findings);
	assert_eq!(
		reasons.len(),
		1,
		"exactly one finding expected: {findings:#?}"
	);
	assert!(
		reasons[0].contains("5432"),
		"reason must name the port; got: {}",
		reasons[0]
	);
}

/// `host_ip: "::"` in the long form: explicit wildcard, fires.
#[test]
fn audit_port_published_on_wildcard_flags_long_with_ipv6_wildcard() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - target: 5432
        published: 5432
        host_ip: "::"
"#;
	let findings = report_for(yaml);
	let reasons = wildcard_reasons(&findings);
	assert_eq!(
		reasons.len(),
		1,
		"exactly one finding expected: {findings:#?}"
	);
	assert!(
		reasons[0].contains("5432"),
		"reason must name the port; got: {}",
		reasons[0]
	);
}

/// `host_ip: "127.0.0.1"` (loopback) is a deliberate decision: must NOT
/// fire.
#[test]
fn audit_port_published_on_wildcard_passes_long_with_loopback_host_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - target: 5432
        published: 5432
        host_ip: 127.0.0.1
"#;
	let findings = report_for(yaml);
	assert!(
		wildcard_reasons(&findings).is_empty(),
		"loopback host_ip must not fire: {findings:#?}"
	);
}

/// Long form with `published` but no `host_ip` is the all-interfaces
/// check's job. Wildcard must NOT fire here.
#[test]
fn audit_port_published_on_wildcard_passes_long_without_host_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - target: 80
        published: 8080
"#;
	let findings = report_for(yaml);
	assert!(
		wildcard_reasons(&findings).is_empty(),
		"no host_ip long form is the all-interfaces check's job; wildcard must \
		 not fire: {findings:#?}"
	);
}

/// Long form with `target` but no `published`: the port is exposed,
/// not published on the host. Neither check fires.
#[test]
fn audit_port_published_on_wildcard_passes_long_without_published() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - target: 5432
        host_ip: 0.0.0.0
"#;
	let findings = report_for(yaml);
	assert!(
		wildcard_reasons(&findings).is_empty(),
		"long form without published must not fire: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// shape aggregation
// ---------------------------------------------------------------------------

/// A port range in `published` with `host_ip: 0.0.0.0` raises one
/// finding carrying the range label verbatim, not one finding per
/// port in the range. Same counting rule as the all-interfaces check.
#[test]
fn audit_port_published_on_wildcard_flags_range_once_with_range_label() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - target: 8080
        published: "8080-8090"
        host_ip: 0.0.0.0
"#;
	let findings = report_for(yaml);
	let reasons = wildcard_reasons(&findings);
	assert_eq!(
		reasons.len(),
		1,
		"a port range is one mapping, so one finding: {findings:#?}"
	);
	assert!(
		reasons[0].contains("8080-8090"),
		"reason must carry the range label verbatim; got: {}",
		reasons[0]
	);
}

/// Mixed list: a wildcard and a specific IP. Only the wildcard fires.
#[test]
fn audit_port_published_on_wildcard_fires_only_for_wildcard_in_mixed_list() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "127.0.0.1:5432:5432"
      - "0.0.0.0:6379:6379"
"#;
	let findings = report_for(yaml);
	let reasons = wildcard_reasons(&findings);
	assert_eq!(
		reasons.len(),
		1,
		"only the wildcard mapping fires: {findings:#?}"
	);
	assert!(
		reasons[0].contains("6379"),
		"the finding must name the wildcard port (6379); got: {}",
		reasons[0]
	);
}

/// A service with no `ports:` field publishes nothing on the host;
/// the check has nothing to attach to and stays silent.
#[test]
fn audit_port_published_on_wildcard_passes_when_no_ports() {
	let yaml = r#"
services:
  web:
    image: nginx
"#;
	let findings = report_for(yaml);
	assert!(
		wildcard_reasons(&findings).is_empty(),
		"no ports: must not fire: {findings:#?}"
	);
}

/// `run_checks` invokes every check once per service, but the
/// wildcard predicate enumerates every flagged port in the whole
/// file. Without per-service filtering, a multi-service file would
/// emit each finding N times. Same shape as the all-interfaces check.
#[test]
fn audit_port_published_on_wildcard_filters_to_the_current_service() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "0.0.0.0:5432:5432"
  cache:
    image: redis
    ports:
      - "0.0.0.0:6379:6379"
"#;
	let findings = report_for(yaml);
	let services: Vec<&str> = findings
		.iter()
		.filter(|f| f.check == "port_published_on_wildcard")
		.map(|f| f.service.as_str())
		.collect();
	assert_eq!(
		services.len(),
		2,
		"one finding per flagged mapping; got: {services:?}"
	);
	let counts: std::collections::HashMap<&str, usize> =
		services.iter().fold(Default::default(), |mut acc, s| {
			*acc.entry(s).or_insert(0) += 1;
			acc
		});
	for svc in ["db", "cache"] {
		assert_eq!(
			counts.get(svc).copied().unwrap_or(0),
			1,
			"service `{svc}` must have exactly one finding; got: {counts:?}"
		);
	}
}
