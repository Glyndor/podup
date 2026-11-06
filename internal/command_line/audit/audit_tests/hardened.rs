use super::*;

// ---------------------------------------------------------------------------
// Top-level run sanity: a fully hardened service has no findings.
// ---------------------------------------------------------------------------

/// Parse `yaml` and return the only service. Panics if zero or >1, every
/// check test targets a single-service file. Kept private to the module so
/// no other file grows a dependency on this shape.
fn single_service(yaml: &str) -> (String, podup::compose::types::Service) {
	let file = parse_str(yaml).expect("compose parses");
	let mut iter = file.services.into_iter();
	let (name, svc) = iter.next().expect("at least one service");
	assert!(iter.next().is_none(), "test should target one service");
	(name, svc)
}

#[test]
fn audit_fully_hardened_service_has_no_findings() {
	// The issue spells the list of keys a hardened service needs; this
	// combines every one of them so a single regression in any check flips
	// this test red.
	let yaml = r#"
services:
  web:
    image: nginx:1.27@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e5e
    read_only: true
    cap_drop: [ALL]
    security_opt: [no-new-privileges:true]
    pids_limit: 200
    mem_limit: 512m
    memswap_limit: 512m
    init: true
    restart: unless-stopped
    cpus: "1"
    healthcheck:
      test: ["CMD", "true"]
      interval: 30s
      x-podman-on-failure: restart
    userns_mode: auto
    environment:
      - LEVEL=info
      - HOSTNAME
"#;
	let (name, svc) = single_service(yaml);
	let report = audit::audit_file(&parse_str(yaml).unwrap(), &[]);
	let findings_for_web: Vec<&Finding> = report
		.findings
		.iter()
		.filter(|f| f.service == name)
		.collect();
	assert!(
		findings_for_web.is_empty(),
		"hardened service produced findings: {findings_for_web:#?}"
	);
	// Sanity: the service really did carry every key we wanted to test, so
	// the empty finding list reflects the check passing each one rather than
	// the checks having nothing to look at. The cap_drop presence in
	// particular guards the most-frequently-regressed check.
	assert!(
		svc.read_only == Some(true),
		"sanity: read_only must be true"
	);
	assert!(
		svc.cap_drop.iter().any(|c| c == "ALL"),
		"sanity: cap_drop must contain ALL"
	);
	assert!(
		!svc.security_opt.is_empty(),
		"sanity: security_opt must be set"
	);
	assert!(svc.pids_limit.is_some(), "sanity: pids_limit must be set");
	assert!(svc.mem_limit.is_some(), "sanity: mem_limit must be set");
	assert!(
		svc.memswap_limit.is_some(),
		"sanity: memswap_limit must be set"
	);
	assert!(svc.init == Some(true), "sanity: init must be true");
	assert!(svc.restart.is_some(), "sanity: restart must be set");
	assert!(svc.cpus.is_some(), "sanity: cpus must be set");
	assert!(svc.healthcheck.is_some(), "sanity: healthcheck must be set");
	assert!(svc.userns_mode.is_some(), "sanity: userns_mode must be set");
}
