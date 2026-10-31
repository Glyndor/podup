use super::*;

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
