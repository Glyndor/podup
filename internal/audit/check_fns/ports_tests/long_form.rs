use super::*;

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
