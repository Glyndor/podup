//! The unit tests of the audit command, one file per subject.
//!
//! The suite was a single `audit_tests.rs` over the 300-line soft limit, so it
//! became a folder: this module keeps the shared scaffolding and the index,
//! and each leaf holds the scenarios of one subject.

// ---------------------------------------------------------------------------
// Test scaffolding
// ---------------------------------------------------------------------------

// Pull the names this test file reaches for into scope so the body reads as
// straight assertions rather than naming the path every time.
#[allow(unused_imports)]
use super::{
	audit_file, audited_containers, render_json, render_json_to, render_table, render_table_to,
	AuditReport, Finding as _, Finding,
};

use crate::audit;
use podup::parse_str;

fn report_for_file(yaml: &str) -> AuditReport {
	audit_file(&parse_str(yaml).unwrap(), &[])
}

mod checks;
mod hardened;
mod hostile;
mod jobs;
mod report;
