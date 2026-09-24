//! Tests for the `no_cpu_limit` check, split from `check_runtime_tests.rs`
//! to keep that file under the audit's 500-line ceiling (`#1894`).
//!
//! Reuses the `report_for` helper from the sibling `checks_tests.rs`
//! module so the assertions stay aligned with the existing per-check
//! pattern.

use super::tests::report_for;

// ---------------------------------------------------------------------------
// no_cpu_limit
// ---------------------------------------------------------------------------

/// Nothing set -> fires.
#[test]
fn audit_no_cpu_limit_flags_when_nothing_set() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	let hits: Vec<&crate::audit::Finding> = findings
		.iter()
		.filter(|f| f.check == "no_cpu_limit")
		.collect();
	assert_eq!(
		hits.len(),
		1,
		"absent cpus/deploy.resources.limits.cpus/cpu_quota must fire once: {findings:#?}"
	);
}

/// `cpus: "0.5"` -> silent. The engine at
/// `internal/engine/container_config/resources.rs::build_resource_limits`
/// parses this to a CFS quota; an audit that ignores the parse fires
/// spuriously on the same input the engine applies a limit to.
#[test]
fn audit_no_cpu_limit_silent_with_cpus() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    cpus: "0.5"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_cpu_limit"),
		"`cpus: \"0.5\"` is a parseable limit: {findings:#?}"
	);
}

/// `deploy.resources.limits.cpus: "1"` -> silent.
#[test]
fn audit_no_cpu_limit_silent_with_deploy_resources_limits_cpus() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    deploy:
      resources:
        limits:
          cpus: "1"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_cpu_limit"),
		"`deploy.resources.limits.cpus` is a parseable limit: {findings:#?}"
	);
}

/// `cpu_quota: 50000` -> silent (the CFS quota form, also a limit per
/// the engine).
#[test]
fn audit_no_cpu_limit_silent_with_cpu_quota() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    cpu_quota: 50000
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_cpu_limit"),
		"`cpu_quota: 50000` is a limit: {findings:#?}"
	);
}

/// `cpu_quota: -1` -> fires. The Docker API treats `-1` as "unlimited"
/// and Podman does the same, so the operator's intent is not the
/// limit they got; the audit must flag it (`#1894`).
#[test]
fn audit_no_cpu_limit_fires_with_cpu_quota_minus_one() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    cpu_quota: -1
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "no_cpu_limit"),
		"`cpu_quota: -1` is the Docker API's unlimited sentinel: {findings:#?}"
	);
}

/// `cpu_quota: 0` -> fires. A zero quota is not a limit (Podman would
/// give the container no CPU time); the audit must flag it (`#1894`).
#[test]
fn audit_no_cpu_limit_fires_with_cpu_quota_zero() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    cpu_quota: 0
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "no_cpu_limit"),
		"`cpu_quota: 0` is not a limit: {findings:#?}"
	);
}

/// `cpuset: "0-1"` alone -> silent. A non-empty `cpuset:` pins the
/// service to named cores, which bounds how many cores it can take
/// even when no quota is in effect. The engine forwards `cpuset:` as
/// a placement constraint, separate from the quota, and the audit
/// agrees by treating it as a limit (`#1894`).
#[test]
fn audit_no_cpu_limit_silent_with_cpuset() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    cpuset: "0-1"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_cpu_limit"),
		"`cpuset: \"0-1\"` pins the service to two cores: {findings:#?}"
	);
}

/// `cpus: "0.5"` with `deploy.resources.limits.cpus: "2"` -> silent.
/// Top-level `cpus:` wins, matching the engine's `build_resource_limits`
/// precedence; the deploy block is ignored once the top level set
/// one. A regression to the `max`-based resolution would wrongly fire
/// here (`#1894`).
#[test]
fn audit_no_cpu_limit_silent_when_top_level_cpus_wins_over_deploy() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    cpus: "0.5"
    deploy:
      resources:
        limits:
          cpus: "2"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_cpu_limit"),
		"top-level cpus: \"0.5\" wins over deploy.resources.limits.cpus: \"2\": {findings:#?}"
	);
}
