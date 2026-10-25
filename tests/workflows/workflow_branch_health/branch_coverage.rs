//! Property 1: both `main` and `develop` are covered, by two jobs that read
//! the same signal, with a planted negative control for the parser that reads
//! their `with:` blocks.
//!
//! Split out from `tests/workflows/workflow_branch_health.rs`; the readers and
//! parsers are inherited via `use super::*`.

use super::*;

#[test]
fn both_protected_branches_have_their_own_health_job() {
	let ci = read("ci.yml");
	let main = ci_yml_health_branch(&ci, "health-main");
	let develop = ci_yml_health_branch(&ci, "health-develop");

	assert_eq!(
		main, "main",
		"health-main passes branch={main:?}, not \"main\". A wrong branch \
		 means the job watches the wrong history, and a red `main` goes \
		 unreported on the only branch where the gap matters."
	);
	assert_eq!(
		develop, "develop",
		"health-develop passes branch={develop:?}, not \"develop\". Same \
		 defect as a wrong value on health-main."
	);
}

#[test]
fn both_jobs_watch_the_same_workflow() {
	let ci = read("ci.yml");
	let main = ci_yml_health_workflow(&ci, "health-main");
	let develop = ci_yml_health_workflow(&ci, "health-develop");
	assert_eq!(
		main, develop,
		"health-main watches {main:?} but health-develop watches \
		 {develop:?}. A different workflow on the two branches means one \
		 of them is reading the wrong signal; the point of the pair is \
		 they read the same one."
	);
}

/// The parser is what the structural tests trust. Pin it on input that
/// differs from today's `ci.yml`, including a job whose `with:` block
/// lists `workflow:` and `branch:` in the wrong order and a comment that
/// names them out of scope. A parser that always returned the same
/// string would pass every assertion above and prove nothing.
#[test]
fn the_branch_parser_reads_what_it_claims() {
	// Both keys in the order the file happens to use.
	let ci_a = "\
jobs:
  health-main:
    uses: ./.github/workflows/reusable-branch-health.yml
    with:
      workflow: ci.yml
      branch: main
  health-develop:
    uses: ./.github/workflows/reusable-branch-health.yml
    with:
      workflow: ci.yml
      branch: develop
";
	assert_eq!(ci_yml_health_branch(ci_a, "health-main"), "main");
	assert_eq!(ci_yml_health_workflow(ci_a, "health-main"), "ci.yml");
	assert_eq!(ci_yml_health_branch(ci_a, "health-develop"), "develop");

	// Reversed order. The parser walks the `with:` block and stops on
	// the first matching key, so the test it backs must be order-blind.
	let ci_b = "\
jobs:
  health-main:
    uses: ./.github/workflows/reusable-branch-health.yml
    with:
      branch: main
      workflow: ci.yml
";
	assert_eq!(ci_yml_health_branch(ci_b, "health-main"), "main");
	assert_eq!(ci_yml_health_workflow(ci_b, "health-main"), "ci.yml");

	// A sibling job that also names `branch:` and `workflow:` in its
	// own `with:` block must not contaminate the answer for the one
	// being asked about.
	let ci_c = "\
jobs:
  freshness-benchmark:
    uses: ./.github/workflows/reusable-schedule-freshness.yml
    with:
      workflow: benchmark.yml
      max-age-days: 65
  health-main:
    uses: ./.github/workflows/reusable-branch-health.yml
    with:
      workflow: ci.yml
      branch: main
  health-develop:
    uses: ./.github/workflows/reusable-branch-health.yml
    with:
      workflow: ci.yml
      branch: develop
";
	assert_eq!(
		ci_yml_health_workflow(ci_c, "health-main"),
		"ci.yml",
		"the parser took the sibling's value"
	);
	assert_eq!(ci_yml_health_branch(ci_c, "health-main"), "main");

	// A comment that names `branch:` and `workflow:` with fake values
	// must not be returned.
	let ci_d = "\
jobs:
  health-main:
    # with:
    #   workflow: fake.yml
    #   branch: nonsense
    uses: ./.github/workflows/reusable-branch-health.yml
    with:
      workflow: ci.yml
      branch: main
";
	assert_eq!(ci_yml_health_workflow(ci_d, "health-main"), "ci.yml");
	assert_eq!(ci_yml_health_branch(ci_d, "health-main"), "main");
}
