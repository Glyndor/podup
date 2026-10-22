use super::*;

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
