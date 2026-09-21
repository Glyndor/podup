//! Per-shape coverage for the `port_published_on_all_interfaces` check.
//!
//! The check shares its predicate with the parse-time port-exposure warning
//! in `internal/compose/diagnostics/ignored_fields.rs`, so every case below
//! is a mirror of one in `port_exposure_tests.rs`: the audit fires iff the
//! warning fires, and vice versa, so the two surfaces cannot drift on a
//! future compose-shape addition (#1835).

use super::tests::report_for;

// ---------------------------------------------------------------------------
// short form
// ---------------------------------------------------------------------------

/// `5432:5432` is the canonical accidental case: no host IP, every
/// interface. The audit must flag it; the same compose file is what the
/// parse-time warning surfaces on `up`/`config`.
#[test]
fn audit_port_published_on_all_interfaces_flags_short_without_host_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "5432:5432"
"#;
	let findings = report_for(yaml);
	let exposure: Vec<&str> = findings
		.iter()
		.filter(|f| f.check == "port_published_on_all_interfaces")
		.map(|f| f.reason.as_str())
		.collect();
	assert_eq!(
		exposure.len(),
		1,
		"exactly one port_exposure finding expected: {findings:#?}"
	);
	assert!(
		findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces" && f.service == "db"),
		"finding must be attached to the `db` service: {findings:#?}"
	);
	assert!(
		exposure[0].contains("5432"),
		"reason must name the port; got: {}",
		exposure[0]
	);
}

/// `127.0.0.1:5432:5432` is the explicit-loopback form: the bind is
/// restricted to the host, so nothing leaks to the network. The audit
/// must agree with the diagnostic warning and stay silent.
#[test]
fn audit_port_published_on_all_interfaces_passes_short_with_loopback_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "127.0.0.1:5432:5432"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces"),
		"loopback must not fire: {findings:#?}"
	);
}

/// `0.0.0.0:5432:5432` is the explicit-all-interfaces form. The
/// operator typed the address, so it is a deliberate decision, the
/// same one docker-compose would honor silently. The diagnostic
/// warning is deliberately silent on this shape (see the comment
/// above `port_published_on_all_interfaces`); the audit must agree.
/// Flagging it would only train the operator to ignore the check.
#[test]
fn audit_port_published_on_all_interfaces_passes_short_with_explicit_all_interfaces_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "0.0.0.0:5432:5432"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces"),
		"explicit 0.0.0.0 is a deliberate decision; must not fire: {findings:#?}"
	);
}

/// A non-loopback but private LAN address is still an explicit bind
/// decision. The diagnostic stays silent; the audit must too.
#[test]
fn audit_port_published_on_all_interfaces_passes_short_with_private_lan_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "192.168.1.10:5432:5432"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces"),
		"private LAN bind is a decision; must not fire: {findings:#?}"
	);
}

/// Container-only short form (`"5432"` with 0 colons) is the
/// short-form mirror of `expose:`: not published on the host, no
/// warning. The audit must stay silent for the same reason.
#[test]
fn audit_port_published_on_all_interfaces_passes_short_container_only() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "5432"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces"),
		"container-only must not fire: {findings:#?}"
	);
}

/// IPv6 short form `[::1]:5432:5432` carries an explicit host IP,
/// so the diagnostic is silent and the audit must be silent too.
#[test]
fn audit_port_published_on_all_interfaces_passes_ipv6_short() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "[::1]:5432:5432"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces"),
		"IPv6 explicit bind must not fire: {findings:#?}"
	);
}

/// Short form with a `/tcp` protocol suffix but no IP is the same
/// accidental shape as `"5432:5432"`. The audit fires once for it,
/// matching the diagnostic warning.
#[test]
fn audit_port_published_on_all_interfaces_flags_short_with_protocol_suffix() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "8080:80/tcp"
"#;
	let findings = report_for(yaml);
	let exposure: Vec<&str> = findings
		.iter()
		.filter(|f| f.check == "port_published_on_all_interfaces")
		.map(|f| f.reason.as_str())
		.collect();
	assert_eq!(
		exposure.len(),
		1,
		"expected exactly one finding: {findings:#?}"
	);
	assert!(
		exposure[0].contains("8080"),
		"reason must name the host port; got: {}",
		exposure[0]
	);
}

// ---------------------------------------------------------------------------
// long form
// ---------------------------------------------------------------------------

/// Long form with `published` but no `host_ip` is the long-form
/// mirror of `5432:5432`: bind on every interface, the audit fires.
#[test]
fn audit_port_published_on_all_interfaces_flags_long_without_host_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - target: 80
        published: 8080
"#;
	let findings = report_for(yaml);
	let exposure: Vec<&str> = findings
		.iter()
		.filter(|f| f.check == "port_published_on_all_interfaces")
		.map(|f| f.reason.as_str())
		.collect();
	assert_eq!(
		exposure.len(),
		1,
		"expected exactly one finding: {findings:#?}"
	);
	assert!(
		exposure[0].contains("8080"),
		"reason must name the published port; got: {}",
		exposure[0]
	);
}

/// Long form with `host_ip: 127.0.0.1` is the long-form mirror of
/// the explicit-loopback short form: restricted to the host, no
/// finding.
#[test]
fn audit_port_published_on_all_interfaces_passes_long_with_host_ip() {
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
		!findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces"),
		"long form with host_ip must not fire: {findings:#?}"
	);
}

/// Long form with `host_ip: 0.0.0.0` is the explicit-all-interfaces
/// long form; the audit must stay silent, matching the diagnostic.
#[test]
fn audit_port_published_on_all_interfaces_passes_long_with_explicit_all_interfaces_host_ip() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - target: 80
        published: 8080
        host_ip: 0.0.0.0
"#;
	let findings = report_for(yaml);
	assert!(
		!findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces"),
		"explicit 0.0.0.0 host_ip is a decision; must not fire: {findings:#?}"
	);
}

/// Long form with `target` but no `published` is the long-form
/// mirror of `expose:`: the port is exposed to peers but not
/// published on the host, so neither the warning nor the audit
/// finding fires.
#[test]
fn audit_port_published_on_all_interfaces_passes_long_without_published() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - target: 5432
"#;
	let findings = report_for(yaml);
	assert!(
		!findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces"),
		"long form without published must not fire: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// ranges, mixed lists, and shape aggregation
// ---------------------------------------------------------------------------

/// A port range in `published` raises one finding carrying the range
/// label verbatim, not one finding per port in the range. The risk
/// is the same shape; multiplying findings across the range adds
/// noise without adding signal.
#[test]
fn audit_port_published_on_all_interfaces_flags_range_once_with_range_label() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - target: 8080
        published: "8080-8090"
"#;
	let findings = report_for(yaml);
	let exposure: Vec<&str> = findings
		.iter()
		.filter(|f| f.check == "port_published_on_all_interfaces")
		.map(|f| f.reason.as_str())
		.collect();
	assert_eq!(
		exposure.len(),
		1,
		"a port range is one mapping, so one finding: {findings:#?}"
	);
	assert!(
		exposure[0].contains("8080-8090"),
		"reason must carry the range label verbatim; got: {}",
		exposure[0]
	);
}

/// Mixed list: one bind with IP, one without. Only the IP-less
/// mapping fires, the same way the diagnostic warning names only the
/// IP-less port.
#[test]
fn audit_port_published_on_all_interfaces_fires_only_for_ip_less_in_mixed_list() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "127.0.0.1:5432:5432"
      - "8080:80"
"#;
	let findings = report_for(yaml);
	let exposure: Vec<&str> = findings
		.iter()
		.filter(|f| f.check == "port_published_on_all_interfaces")
		.map(|f| f.reason.as_str())
		.collect();
	assert_eq!(
		exposure.len(),
		1,
		"only the IP-less port fires: {findings:#?}"
	);
	assert!(
		exposure[0].contains("8080"),
		"the finding must name the IP-less port (8080); got: {}",
		exposure[0]
	);
}

/// A service with no `ports:` field publishes nothing on the host;
/// the check has nothing to attach to and stays silent.
#[test]
fn audit_port_published_on_all_interfaces_passes_when_no_ports() {
	let yaml = r#"
services:
  web:
    image: nginx
"#;
	let findings = report_for(yaml);
	assert!(
		!findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces"),
		"no ports: must not fire: {findings:#?}"
	);
}

/// `expose:` alone (no `ports:`) is reachable to peers on the same
/// compose network but not on the host, the same case as no ports
/// at all. The audit must stay silent.
#[test]
fn audit_port_published_on_all_interfaces_passes_on_expose_only() {
	let yaml = r#"
services:
  db:
    image: postgres
    expose:
      - "5432"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings
			.iter()
			.any(|f| f.check == "port_published_on_all_interfaces"),
		"expose only must not fire: {findings:#?}"
	);
}

/// `run_checks` invokes every check once per service, but the
/// port-exposure predicate enumerates every flagged port in the
/// whole file. Without per-service filtering, a multi-service file
/// would emit each finding N times (once per service visit), so a
/// two-service compose file with one bad port each would report two
/// findings per service. The check filters to the current service
/// before emitting, so the count stays at exactly one finding per
/// flagged mapping.
#[test]
fn audit_port_published_on_all_interfaces_filters_to_the_current_service() {
	let yaml = r#"
services:
  db:
    image: postgres
    ports:
      - "5432:5432"
  cache:
    image: redis
    ports:
      - "6379:6379"
"#;
	let findings = report_for(yaml);
	let exposure: Vec<&str> = findings
		.iter()
		.filter(|f| f.check == "port_published_on_all_interfaces")
		.map(|f| f.service.as_str())
		.collect();
	assert_eq!(
		exposure.len(),
		2,
		"one finding per flagged mapping; got: {exposure:?}"
	);
	let counts: std::collections::HashMap<&str, usize> =
		exposure.iter().fold(Default::default(), |mut acc, s| {
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
