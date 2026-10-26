//! Each health caller carries a job-level `if:` that takes it out of the push
//! run of the branch it watches, with a planted negative control for the shape
//! that decides it.
//!
//! Split out from `tests/repository/workflows/workflow_branch_health.rs`; the reader, the
//! `with.branch` parser and the job-id list are inherited via `use super::*`.

use super::*;

// Each health caller must carry a job-level `if:` that takes it out of
// the push run of the branch it watches. The latching defect measured
// on 2026-09-19: three pulls merged into `develop` seconds apart, each
// push cancelling the CI run of the push before it, and `branch health
// (develop)` was a job in each of those runs. A run that includes the
// job reads itself, fails on the red run before it, and makes its own
// run red, which the next push then reads.
#[test]
fn each_health_job_carries_an_if_that_excludes_its_own_push_run() {
	let ci = read("ci.yml");
	for job_id in HEALTH_JOBS {
		let if_value = ci_yml_health_if_value(&ci, job_id).unwrap_or_else(|| {
			panic!(
				"`{job_id}` in ci.yml has no job-level `if:`. The latching \
				 fix measured on 2026-09-19 keeps this caller out of the \
				 push run of the branch it watches; a missing `if:` puts \
				 it back in, and the next push reads this run as red."
			)
		});
		let branch = ci_yml_health_branch(&ci, job_id);
		let expected = expected_health_if(&branch);
		assert_eq!(
			if_value, expected,
			"`{job_id}`'s `if:` ({if_value:?}) is not exactly `{expected}`. \
			 A weaker `contains` would accept `... || true`, which is true \
			 on every push of {branch} and puts the caller back into the \
			 run it is supposed to skip; matching the whole expression \
			 is what stops that."
		);
	}
}

/// Indices of a job's body lines: (start, end). Same scope rule as the
/// `with:` parsers above.
fn ci_yml_job_body_range(workflow: &str, job_id: &str) -> (usize, usize) {
	let lines: Vec<&str> = workflow.lines().collect();
	let job_idx = lines
		.iter()
		.position(|l| l.trim_start_matches(' ').trim_start() == format!("{job_id}:"))
		.unwrap_or_else(|| panic!("ci.yml has a `{job_id}` job"));
	let job_indent = lines[job_idx].len() - lines[job_idx].trim_start().len();
	let job_end = lines[job_idx + 1..]
		.iter()
		.position(|l| {
			let trimmed = l.trim_start();
			if trimmed.is_empty() || trimmed.starts_with('#') {
				return false;
			}
			let indent = l.len() - trimmed.len();
			indent <= job_indent
		})
		.map(|p| p + job_idx + 1)
		.unwrap_or(lines.len());
	(job_idx + 1, job_end)
}

/// `if:` value, or `None` when the job has none.
fn ci_yml_health_if_value(workflow: &str, job_id: &str) -> Option<String> {
	let lines: Vec<&str> = workflow.lines().collect();
	let (start, end) = ci_yml_job_body_range(workflow, job_id);
	for line in &lines[start..end] {
		let trimmed = line.trim_start();
		if trimmed.is_empty() || trimmed.starts_with('#') {
			continue;
		}
		if let Some(rest) = trimmed.strip_prefix("if:") {
			return Some(rest.trim().to_string());
		}
	}
	None
}

/// The `if:` line that takes a health caller out of the push run of the
/// branch it watches, with the branch half taken from that job's own
/// `with.branch`, behind the merge-queue exclusion: the watchers do not
/// run in a merge group, which answers a question about the queue and
/// not about the branch they watch. Anything else is rejected so a
/// future refactor that flips the conjunction or appends `|| true` does
/// not silently disarm the latching fix.
fn expected_health_if(branch: &str) -> String {
	format!(
		"github.event_name != 'merge_group' && \
		 (github.event_name != 'push' || github.ref_name != '{branch}')"
	)
}

/// Whether a job's `if:` is exactly the self-branching shape for its
/// own `with.branch`. Equality, not contains: a copy-paste that leaves
/// `develop` guarding `main` returns false here, and so does an
/// expression that has the right substrings plus `|| true` (which
/// re-arms the latching defect on every push of the watched branch).
fn has_self_branching_if(workflow: &str, job_id: &str) -> bool {
	let if_value = match ci_yml_health_if_value(workflow, job_id) {
		Some(v) => v,
		None => return false,
	};
	let branch = ci_yml_health_branch(workflow, job_id);
	if_value == expected_health_if(&branch)
}

// Pin the assertion on input that differs from today's `ci.yml`: a
// check that always returned true would satisfy the structural
// assertion above and prove nothing. The positive case is covered by
// the production test on the real `ci.yml`.
#[test]
fn the_self_branching_if_catches_a_missing_or_swapped_line() {
	// `if:` line removed: the job is back inside the push run it is
	// supposed to skip, which is the latching shape.
	let ci_missing = "\
jobs:
  health-main:
    uses: ./.github/workflows/reusable-branch-health.yml
    with:
      workflow: ci.yml
      branch: main
";
	assert!(
		!has_self_branching_if(ci_missing, "health-main"),
		"a job with no `if:` line was treated as self-branching"
	);

	// Branches swapped: `with:` says `main`, `if:` says `develop`.
	let ci_swapped = "\
jobs:
  health-main:
    if: github.event_name != 'merge_group' && (github.event_name != 'push' || github.ref_name != 'develop')
    uses: ./.github/workflows/reusable-branch-health.yml
    with:
      workflow: ci.yml
      branch: main
";
	assert!(
		!has_self_branching_if(ci_swapped, "health-main"),
		"a job whose `if:` names the other branch was accepted"
	);

	// The exact shape with `|| true` appended: the conjunction is true
	// on every push of the watched branch, which is the latching
	// defect the `if:` exists to prevent. Equality (not `contains`)
	// is what catches it.
	let ci_true = "\
jobs:
  health-main:
    if: github.event_name != 'merge_group' && (github.event_name != 'push' || github.ref_name != 'main') || true
    uses: ./.github/workflows/reusable-branch-health.yml
    with:
      workflow: ci.yml
      branch: main
";
	assert!(
		!has_self_branching_if(ci_true, "health-main"),
		"an `if:` of the right shape plus `|| true` is true on every \
		 push of main and would re-arm the latching defect; equality \
		 is what stops that"
	);
}
