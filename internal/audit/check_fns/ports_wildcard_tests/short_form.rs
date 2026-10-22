use super::*;

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
