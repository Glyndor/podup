use super::*;

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
