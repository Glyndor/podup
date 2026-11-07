//! The warning pass that follows a load is not charged to a load around it.
//!
//! After the files of a project are parsed, a second pass re-reads each of them
//! to warn about keys the typed model dropped, and that pass interpolates again.
//! If it counted against the budget of a project that is loading on the same
//! thread (this one was called from a callback of it), a file that fits would
//! run past the limit there and the pass would end without a word.

use crate::substitute::project_budget;

const MIB: usize = 1024 * 1024;

#[test]
fn the_warning_pass_counts_each_file_alone_inside_another_load() {
	let dir = tempfile::tempdir().unwrap();
	// A name no other test sets in the process environment, which wins over `.env`.
	std::fs::write(
		dir.path().join(".env"),
		format!("DIAG_BUDGET_PAD={}\n", "x".repeat(MIB)),
	)
	.unwrap();
	let file = dir.path().join("compose.yaml");
	std::fs::write(
		&file,
		"services:\n  s:\n    image: alpine\n    command: [\"${DIAG_BUDGET_PAD}\", \"${DIAG_BUDGET_PAD}\"]\n    \
		 volumes:\n      - type: bind\n        source: /tmp\n        target: /x\n        \
		 bind:\n          propagaton: rshared\n",
	)
	.unwrap();

	// Another project is 15 MiB in on this thread.
	let _outer = project_budget::Scope::open();
	project_budget::record(15 * MIB);

	let warnings = crate::compose::diagnostics::collect_raw_nested_warnings(
		std::slice::from_ref(&file),
		&[],
		true,
		None,
		None,
	);
	assert!(
		warnings.iter().any(|w| w.contains("propagaton")),
		"no warning about the mistyped option: {warnings:?}"
	);
	assert_eq!(project_budget::spent(), 15 * MIB, "the outer count is back");
}
