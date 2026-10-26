//! Property 3: the check is advisory, both files say so, and both job ids
//! keep their name.
//!
//! Split out from `tests/repository/workflows/workflow_branch_health.rs`; the reader and the
//! job-id list are inherited via `use super::*`.

use super::*;

#[test]
fn the_check_is_advisory_and_the_file_says_so() {
	let reusable = read("reusable-branch-health.yml");
	let ci = read("ci.yml");

	// The reusable carries the warning in its own header, where somebody
	// wiring required checks against it would look first. The wording is
	// deliberately emphatic ("NOT A REQUIRED STATUS CHECK"); the assertion
	// is case-insensitive so future rewording that drops the caps still
	// satisfies it, as long as the warning is present.
	let reusable_warns = reusable.lines().any(|l| {
		let lower = l.to_lowercase();
		lower.contains("required status check") && lower.contains("not ")
	});
	assert!(
		reusable_warns,
		"reusable-branch-health.yml no longer carries the comment that \
		 warns against making it a required status check. A red branch \
		 would then block the pull request that repairs it, which is the \
		 failure this check exists to surface."
	);

	// `ci.yml` carries a matching comment beside the job ids. Without
	// both copies, a reviewer wiring the check required has to read the
	// reusable to learn the rule, and the place they would actually look
	// is the call site.
	let ci_warns = ci
		.lines()
		.any(|l| l.contains("MUST NOT BECOME A REQUIRED STATUS CHECK"));
	assert!(
		ci_warns,
		"ci.yml no longer carries the `MUST NOT BECOME A REQUIRED STATUS \
		 CHECK` comment beside the branch-health jobs. That copy is what \
		 stops a reviewer from marking them required at the call site, \
		 which is where required checks are wired."
	);

	// Both jobs must keep their ids. A required check is matched by name,
	// so a rename creates a phantom the ruleset still requires. The ids
	// below are part of the wire format.
	for id in HEALTH_JOBS {
		assert!(
			ci.contains(&format!("{id}:")),
			"ci.yml is missing the `{id}` job. Renaming a job that is \
			 intended as advisory is fine; the warning here is for the \
			 branch-health pair, whose job ids are part of the file's \
			 contract with this test."
		);
	}
}
