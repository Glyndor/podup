use super::super::tests::report_for;

// ---------------------------------------------------------------------------
// no_pids_limit
// ---------------------------------------------------------------------------

#[test]
fn audit_no_pids_limit_flags_when_unset() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "no_pids_limit"),
		"unset pids_limit must fire: {findings:#?}"
	);
}

#[test]
fn audit_no_pids_limit_passes_when_set() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    pids_limit: 200
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_pids_limit"),
		"pids_limit: 200 must pass: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// no_memory_limit
// ---------------------------------------------------------------------------

#[test]
fn audit_no_memory_limit_flags_when_unset() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "no_memory_limit"),
		"unset memory limit must fire: {findings:#?}"
	);
}

#[test]
fn audit_no_memory_limit_passes_when_set_on_mem_limit() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    mem_limit: 512m
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_memory_limit"),
		"mem_limit must pass: {findings:#?}"
	);
}

#[test]
fn audit_no_memory_limit_passes_when_set_on_deploy_resources() {
	// The check accepts either location; `deploy.resources.limits.memory`
	// covers the swarm-style shape that some compose files adopted as
	// rootless Podman grew up.
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    deploy:
      resources:
        limits:
          memory: 256M
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_memory_limit"),
		"deploy.resources.limits.memory must pass: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// #1743: audit must read the resolved value the engine will use
// ---------------------------------------------------------------------------
//
// Each of the three tests below targets one of the issue's findings and is
// built from a compose file that `audit --strict` accepted on the unfixed
// tree but must not afterwards. They are the issue's acceptance criterion,
// and failed on the unfixed tree. Keeping them in a single
// section so the file's `grep "1743"` lands the reviewer on the lot.

/// `mem_limit: not-a-size` keeps the compose field non-empty, so an
/// `.is_none()` audit reads it as a limit, but `parse_memory` returns
/// `None` and the runtime applies no limit. The audit must agree.
#[test]
fn audit_no_memory_limit_flags_an_unparseable_limit() {
	let report =
		report_for("services:\n  web:\n    image: alpine:3.20\n    mem_limit: not-a-size\n");
	assert!(
		report.iter().any(|f| f.check == "no_memory_limit"),
		"a mem_limit the runtime cannot parse must still fire: {report:#?}"
	);
}
