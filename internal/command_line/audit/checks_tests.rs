//! The `report_for` helper the per-check tests share. Each check is covered by
//! exactly two names, `<check>_flags_when_bad` and `<check>_passes_when_good`,
//! in the test file next to the check, under `check_fns/`.

use podup::parse_str;

use super::*;

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Run every check against `yaml` and return the flat list of findings.
/// The project directory is the fixed one `audit_file` uses; [`report_in`]
/// takes another.
pub(super) fn report_for(yaml: &str) -> Vec<Finding> {
	let file = parse_str(yaml).expect("compose parses");
	crate::audit::audit_file(&file, &[]).findings
}

/// [`report_for`] with the project directory set to `base_dir`, for the
/// checks that resolve host paths against it.
pub(super) fn report_in(yaml: &str, base_dir: &std::path::Path) -> Vec<Finding> {
	let file = parse_str(yaml).expect("compose parses");
	let containers = crate::audit::audited_containers(&file);
	crate::audit::audit_containers(&file, base_dir, &containers, &[]).findings
}

/// The home directory the engine expands `~` to while the tests run. The
/// tests that need it fail rather than pass silently when it is unset.
pub(super) fn test_home() -> String {
	std::env::var("HOME")
		.ok()
		.filter(|h| !h.is_empty() && h != "/")
		.expect("HOME must name a directory for the tests that expand ~")
}
