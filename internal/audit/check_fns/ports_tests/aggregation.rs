use super::*;

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
