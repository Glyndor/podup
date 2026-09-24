//! Per-check tests for the five runtime checks from #1894.
//!
//! Each check has at least one firing case and at least one non-firing
//! case of the same shape just inside the rule, asserting the finding's
//! `id` (not only the count) so a regression that fires the wrong
//! check id is caught here rather than by a downstream consumer
//! diffing the JSON. Per the spec: a firing case inverts or deletes
//! the check's condition and goes red; the non-firing case pins the
//! shape the audit must NOT flag.
//!
//! Reuses the `report_for` helper from the sibling `checks_tests.rs`
//! module so the assertions stay aligned with the existing per-check
//! pattern.

use super::tests::report_for;

// ---------------------------------------------------------------------------
// no_restart_policy
// ---------------------------------------------------------------------------

/// Neither `restart:` nor `deploy.restart_policy:` set -> the check fires.
#[test]
fn audit_no_restart_policy_flags_when_neither_is_set() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	let hits: Vec<&crate::audit::Finding> = findings
		.iter()
		.filter(|f| f.check == "no_restart_policy")
		.collect();
	assert_eq!(
		hits.len(),
		1,
		"absent restart and absent deploy.restart_policy must fire once: {findings:#?}"
	);
	assert_eq!(hits[0].service, "web");
}

/// `restart: unless-stopped` is a deliberate choice -> silent.
#[test]
fn audit_no_restart_policy_silent_with_unless_stopped() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    restart: unless-stopped
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_restart_policy"),
		"`restart: unless-stopped` is a deliberate choice: {findings:#?}"
	);
}

/// `restart: "no"` is the operator opting out of restarts on purpose ->
/// silent. The issue pins this shape explicitly.
#[test]
fn audit_no_restart_policy_silent_with_explicit_no() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    restart: "no"
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_restart_policy"),
		"`restart: \"no\"` is an explicit opt-out: {findings:#?}"
	);
}

/// `deploy.restart_policy` alone (no top-level `restart:`) -> silent.
#[test]
fn audit_no_restart_policy_silent_with_only_deploy_restart_policy() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    deploy:
      restart_policy:
        condition: any
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_restart_policy"),
		"`deploy.restart_policy` alone is a deliberate choice: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// no_init
// ---------------------------------------------------------------------------

/// No `init:` at all (the compose default is `false`) -> fires.
#[test]
fn audit_no_init_flags_when_absent() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	let hits: Vec<&crate::audit::Finding> =
		findings.iter().filter(|f| f.check == "no_init").collect();
	assert_eq!(hits.len(), 1, "absent init must fire once: {findings:#?}");
	assert_eq!(hits[0].service, "web");
}

/// `init: false` is the explicit form of "PID 1 is the app" -> fires the
/// same way an absent key does.
#[test]
fn audit_no_init_flags_when_false() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    init: false
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "no_init"),
		"`init: false` is the explicit no-init form: {findings:#?}"
	);
}

/// `init: true` is the operator opting in -> silent.
#[test]
fn audit_no_init_silent_when_true() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    init: true
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_init"),
		"`init: true` must pass: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// no_health_action
// ---------------------------------------------------------------------------

/// A non-disabled healthcheck with no extension -> fires. Use a
/// `disable: false` to assert that explicit `false` does NOT count as
/// "disabled".
#[test]
fn audit_no_health_action_flags_when_healthcheck_set_without_extension() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    healthcheck:
      test: ["CMD", "true"]
      interval: 30s
      timeout: 5s
      retries: 3
"#;
	let findings = report_for(yaml);
	let hits: Vec<&crate::audit::Finding> = findings
		.iter()
		.filter(|f| f.check == "no_health_action")
		.collect();
	assert_eq!(
		hits.len(),
		1,
		"healthcheck without x-podman-on-failure must fire once: {findings:#?}"
	);
	assert_eq!(hits[0].service, "web");
}

/// `x-podman-on-failure: restart` wires an action -> silent.
#[test]
fn audit_no_health_action_silent_with_podman_on_failure() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    healthcheck:
      test: ["CMD", "true"]
      interval: 30s
      x-podman-on-failure: restart
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_health_action"),
		"`x-podman-on-failure: restart` wires an action: {findings:#?}"
	);
}

/// No `healthcheck:` at all -> silent (no probe means no detected
/// unhealthy state, no need for an action).
#[test]
fn audit_no_health_action_silent_without_healthcheck() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_health_action"),
		"no healthcheck means nothing to act on: {findings:#?}"
	);
}

/// `disable: true` -> silent (the operator opted out of the healthcheck
/// entirely, so the action question is moot).
#[test]
fn audit_no_health_action_silent_when_disabled() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    healthcheck:
      disable: true
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_health_action"),
		"`disable: true` opts out of the check: {findings:#?}"
	);
}

/// `test: ["NONE"]` -> silent (same outcome as `disable: true`).
#[test]
fn audit_no_health_action_silent_with_none_test() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    healthcheck:
      test: ["NONE"]
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_health_action"),
		"`test: [\"NONE\"]` opts out of the check: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// swap_unbounded
// ---------------------------------------------------------------------------

/// `mem_limit: 256m` with no `memswap_limit` -> fires (Podman defaults
/// swap to twice the memory limit at #1894's measurement).
#[test]
fn audit_swap_unbounded_flags_with_mem_limit_alone() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    mem_limit: 256m
"#;
	let findings = report_for(yaml);
	let hits: Vec<&crate::audit::Finding> = findings
		.iter()
		.filter(|f| f.check == "swap_unbounded")
		.collect();
	assert_eq!(
		hits.len(),
		1,
		"`mem_limit` alone must fire once: {findings:#?}"
	);
	assert_eq!(hits[0].service, "web");
}

/// `memswap_limit: -1` (Podman's "unlimited swap" sentinel) -> fires.
#[test]
fn audit_swap_unbounded_flags_with_memswap_unlimited() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    mem_limit: 256m
    memswap_limit: "-1"
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "swap_unbounded"),
		"`memswap_limit: -1` is unlimited swap: {findings:#?}"
	);
}

/// `memswap_limit: 512m` while `mem_limit: 256m` -> fires (different
/// values allow paging beyond the memory limit).
#[test]
fn audit_swap_unbounded_flags_when_memswap_exceeds_mem_limit() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    mem_limit: 256m
    memswap_limit: 512m
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "swap_unbounded"),
		"`memswap_limit: 512m` with `mem_limit: 256m` must fire: {findings:#?}"
	);
}

/// `memswap_limit` equal to the memory limit -> silent.
#[test]
fn audit_swap_unbounded_silent_when_memswap_equals_mem_limit() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    mem_limit: 256m
    memswap_limit: 256m
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "swap_unbounded"),
		"`memswap_limit: 256m` equals `mem_limit: 256m`: {findings:#?}"
	);
}

/// No memory limit at all -> silent (this is `no_memory_limit`'s
/// finding, not this one's; reporting both would double-count the
/// same risk).
#[test]
fn audit_swap_unbounded_silent_with_no_memory_limit() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "swap_unbounded"),
		"no memory limit is no_memory_limit's finding, not this one's: {findings:#?}"
	);
}

/// `mem_limit: not-a-size` with no `memswap_limit` -> silent (no limit
/// in effect, same as absent `mem_limit`).
#[test]
fn audit_swap_unbounded_silent_with_unparseable_mem_limit() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    mem_limit: not-a-size
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "swap_unbounded"),
		"unparseable mem_limit is no limit; not swap_unbounded's finding: {findings:#?}"
	);
}

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
