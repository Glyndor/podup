//! The `report_for` helper the per-check tests share. Each check is covered by
//! exactly two names, `<check>_flags_when_bad` and `<check>_passes_when_good`,
//! in the test file next to the check, under `check_fns/`.

use podup::parse_str;

use super::*;

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Run every check against `yaml` and return the flat list of findings.
/// The checks today are all per-service, so callers only need this entry.
pub(super) fn report_for(yaml: &str) -> Vec<Finding> {
	let file = parse_str(yaml).expect("compose parses");
	crate::audit::audit_file(&file, &[]).findings
}
