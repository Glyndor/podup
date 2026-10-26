//! Tests for the `no_health_action` check, split from
//! `check_runtime_tests.rs` to keep that file under the audit's 500-line
//! ceiling (`#1894`).
//!
//! Reuses the `report_for` helper from the sibling `checks_tests.rs`
//! module so the assertions stay aligned with the existing per-check
//! pattern.

use super::tests::report_for;

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

/// An invalid `x-podman-on-failure` value -> silent. Validation
/// rejects the value before audit runs (`PodmanError::Field`); the
/// audit must not also fire `no_health_action`, which would report a
/// finding for an error validation already owns (`#1894`).
#[test]
fn audit_no_health_action_silent_with_invalid_podman_on_failure() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    healthcheck:
      test: ["CMD", "true"]
      interval: 30s
      x-podman-on-failure: bogus-action
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_health_action"),
		"invalid x-podman-on-failure is validate's job; audit must stay silent: {findings:#?}"
	);
}
