//! Tests for the `swap_unbounded` check, split from
//! `check_runtime_tests.rs` to keep that file under the audit's 500-line
//! ceiling (`#1894`).
//!
//! Reuses the `report_for` helper from the sibling `checks_tests.rs`
//! module so the assertions stay aligned with the existing per-check
//! pattern.

use super::tests::report_for;

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

/// Top-level `mem_limit` wins over `deploy.resources.limits.memory`,
/// matching the engine's `build_resource_limits` precedence. With the
/// deploy block ignored, `memswap_limit: 256m` matches the top-level
/// cap and the check stays silent (`#1894`).
#[test]
fn audit_swap_unbounded_silent_when_top_level_wins_over_deploy() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    mem_limit: 256m
    deploy:
      resources:
        limits:
          memory: 512m
    memswap_limit: 256m
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "swap_unbounded"),
		"top-level mem_limit wins: deploy.resources.limits.memory: 512m is ignored, \
		 and memswap_limit: 256m matches the effective cap: {findings:#?}"
	);
}

/// Same shape as the silent test above, but the swap value differs
/// from the top-level memory limit. The check must fire because the
/// effective cap is the top-level 256m and the swap exceeds it
/// (`#1894`).
#[test]
fn audit_swap_unbounded_fires_when_top_level_wins_and_swap_differs() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    mem_limit: 256m
    deploy:
      resources:
        limits:
          memory: 512m
    memswap_limit: 512m
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "swap_unbounded"),
		"top-level mem_limit wins, swap exceeds it: {findings:#?}"
	);
}
