//! Per-check tests for the `no_restart_policy` and `no_init` checks from
//! #1894. The `no_health_action`, `swap_unbounded`, and `no_cpu_limit`
//! checks live in sibling files (`check_runtime_health_tests.rs`,
//! `check_runtime_swap_tests.rs`, `check_runtime_cpu_tests.rs`) so this
//! file stays under the audit's 500-line ceiling.
//!
//! Each check has at least one firing case and at least one non-firing
//! case of the same shape just inside the rule, asserting the finding's
//! `id` (not only the count) so a regression that fires the wrong
//! check id is caught here rather than by a downstream consumer
//! diffing the JSON.
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
