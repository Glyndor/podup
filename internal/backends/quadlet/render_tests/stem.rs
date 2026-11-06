use super::*;

// --- unit stems -----------------------------------------------------------

#[test]
fn safe_unit_stem_neutralizes_traversal_and_control_chars() {
	assert_eq!(safe_unit_stem("web"), "web");
	assert_eq!(safe_unit_stem("db-data_1.x"), "db-data_1.x");
	assert_eq!(safe_unit_stem("../../etc/passwd"), "_.._.._etc_passwd");
	assert_eq!(safe_unit_stem("/abs"), "_abs");
	assert_eq!(safe_unit_stem(".hidden"), "_.hidden");
	assert_eq!(safe_unit_stem(""), "_");
	assert!(!safe_unit_stem("a\nb").contains('\n'));
}

#[test]
fn safe_unit_stem_strips_leading_dash() {
	// A name starting with `-`/`--` must not yield a file name beginning with
	// a dash (a globbing/flag-injection hazard for downstream tooling).
	assert_eq!(safe_unit_stem("--foo"), "_--foo");
	assert_eq!(safe_unit_stem("-x"), "_-x");
	assert!(!safe_unit_stem("--foo").starts_with('-'));
}

#[test]
fn unit_stem_is_project_prefixed() {
	assert_eq!(unit_stem("proj", "web"), "proj-web");
	// A leading dash in the project still cannot produce a dash-leading stem.
	assert!(!unit_stem("-p", "web").starts_with('-'));
}
