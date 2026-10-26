//! Per-shape coverage for the `port_published_on_wildcard` check.
//!
//! Mirrors the structure of `ports_tests.rs`: every shape
//! that the `ports_published_on_wildcard` predicate in
//! `internal/parsing/compose/diagnostics/port_exposure.rs` accepts or rejects
//! has a test here. The opt-in gate is exercised by `list_checks_tests
//! ::opt_in_check_does_not_fire_when_its_flag_is_absent` and the binary
//! test `tests/audit_exit_codes.rs`; this file owns the per-shape
//! `#1881` matrix itself.

use podup::parse_str;

use crate::audit::{audit_file, Finding};

/// Run every check against `yaml` with `--wildcard-binds` enabled and
/// return the flat list of findings. Every test in this file passes the
/// flag on, so the wildcard findings surface; the per-shape assertions
/// then filter by `check` to ignore the unrelated findings the same
/// compose file raises.
fn report_for(yaml: &str) -> Vec<Finding> {
	let file = parse_str(yaml).expect("compose parses");
	audit_file(&file, &["--wildcard-binds"]).findings
}

fn wildcard_reasons(findings: &[Finding]) -> Vec<&str> {
	findings
		.iter()
		.filter(|f| f.check == "port_published_on_wildcard")
		.map(|f| f.reason.as_str())
		.collect()
}

mod aggregation;
mod long_form;
mod short_form;
