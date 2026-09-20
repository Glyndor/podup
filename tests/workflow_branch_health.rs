//! The branch-health guard must cover both protected branches, query the
//! newest COMPLETED run, and stay advisory rather than required.
//!
//! The reusable lives next to `reusable-schedule-freshness.yml` and is
//! called from `ci.yml` for `main` and `develop`. The freshness watchers
//! cannot catch this defect: they read the newest SUCCESSFUL scheduled
//! run, so a failing run is missing from their answer by construction.
//! On 2026-09-06 three distribution channels sat with `main` red for
//! twenty minutes after a pull request merged while its suite was still
//! reporting, and the only reason anybody knew is that somebody went
//! looking. This file asserts the shape of the fix on every pull request,
//! so the gap is closed at the moment it would otherwise re-open.
//!
//! Four properties, one test each, plus a planted negative control:
//!
//! 1. Both `main` and `develop` are covered. A single job that watched
//!    both would emit one check name covering two branches, and a check
//!    name that does not move is the property required checks depend on
//!    (Glyndor/podup#1552). The two jobs stay separate.
//! 2. The query asks for `status=completed`. Asking for the newest run of
//!    any status would report an in-progress run as a failure, and the
//!    pull request repairing the branch would block itself with a
//!    verdict nobody has finished reaching.
//! 3. The check is advisory. Reading the live rulesets requires the
//!    GitHub Settings UI, and `gh api` is forbidden here, so the
//!    property asserted is the comment beside the job explaining why it
//!    must not be marked required. The reason is also beside the
//!    `uses:` inside the reusable. Both copies must stay.
//! 4. The parser reads what it claims. A planted YAML string with the
//!    shape being asserted exercises the parser; a parser that always
//!    returned true would satisfy the structural assertions and prove
//!    nothing.
//!
//! The conclusion logic itself lives in
//! `.github/scripts/check-branch-conclusion.sh` and is exercised against
//! planted API answers in `tests/shell/branch-health-conclusion.test.sh`,
//! which is the only assertion of behavior this repository can make
//! without `gh`.

use std::fs;
use std::path::Path;

fn read(name: &str) -> String {
	let path = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join(".github/workflows")
		.join(name);
	fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} is readable: {e}"))
}

/// Pull the value of a `branch:` input out of a job's `with:` block in
/// `ci.yml`. Scoped to the named job so a future drift between the two
/// `health-*` jobs is not papered over.
///
/// Same shape as `tests/workflow_toolchain_pin/deb_pair.rs`'s scoped
/// parsers: the scan starts at the `with:` block under the named job
/// (jobs live at the same indent in `ci.yml`) and stops when a sibling
/// key at the same or lower indent appears.
fn ci_yml_health_branch(workflow: &str, job_id: &str) -> String {
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

	let with_idx = lines[job_idx + 1..job_end]
		.iter()
		.position(|l| l.trim() == "with:")
		.unwrap_or_else(|| panic!("`{job_id}` in ci.yml has a `with:` block"));
	let with_abs = job_idx + 1 + with_idx;

	for line in &lines[with_abs + 1..job_end] {
		let trimmed = line.trim_start();
		if trimmed.is_empty() {
			continue;
		}
		if let Some(rest) = trimmed.strip_prefix("branch:") {
			return rest.trim().trim_matches('"').to_string();
		}
	}
	panic!("`{job_id}` in ci.yml passes a `branch:` input");
}

/// Pull the workflow file name out of the same `with:` block.
fn ci_yml_health_workflow(workflow: &str, job_id: &str) -> String {
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

	let with_idx = lines[job_idx + 1..job_end]
		.iter()
		.position(|l| l.trim() == "with:")
		.unwrap_or_else(|| panic!("`{job_id}` in ci.yml has a `with:` block"));
	let with_abs = job_idx + 1 + with_idx;

	for line in &lines[with_abs + 1..job_end] {
		let trimmed = line.trim_start();
		if trimmed.is_empty() {
			continue;
		}
		if let Some(rest) = trimmed.strip_prefix("workflow:") {
			return rest.trim().trim_matches('"').to_string();
		}
	}
	panic!("`{job_id}` in ci.yml passes a `workflow:` input");
}

/// Job ids that have to keep their name. A required check is matched by
/// name, so a renamed job is a phantom the ruleset still requires.
const HEALTH_JOBS: &[&str] = &["health-main", "health-develop"];

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

#[test]
fn the_reusable_checks_out_the_tree_before_running_the_script() {
	// The step runs a script from this repository. Without a checkout the job
	// dies with `No such file or directory` and exit 127, which reads like a
	// broken script rather than a missing step; it happened on the first run of
	// this workflow, and the same omission cost two of the distribution
	// channels a red job earlier the same day.
	let reusable = read("reusable-branch-health.yml");
	let checkout = reusable
		.lines()
		.position(|l| l.contains("actions/checkout@"))
		.expect("the health job checks out the repository");
	let script = reusable
		.lines()
		.position(|l| l.contains("check-branch-conclusion.sh"))
		.expect("the health job runs the conclusion script");
	assert!(
		checkout < script,
		"the checkout has to come before the script that needs the tree"
	);
	assert!(
		reusable.contains("persist-credentials: false"),
		"nothing here writes, so the checkout must not leave a credential behind"
	);
}

#[test]
fn the_reusable_asks_for_completed_runs_only() {
	let reusable = read("reusable-branch-health.yml");
	// The query must be a literal `status=completed` substring on a line
	// that runs `gh api`, paired with `per_page=30`. `per_page=1` returns
	// whatever item lands at index 0, which the API does not guarantee
	// is the newest run, which gave the schedule-freshness gate two
	// false reds measured in 2026-09-19; the script sorts the page
	// itself, so the page is a buffer to scan rather than a shortcut.
	let has_query = reusable.lines().any(|l| {
		let trimmed = l.trim_start();
		if trimmed.starts_with('#') {
			return false;
		}
		trimmed.contains("status=completed") && trimmed.contains("per_page=30")
	});
	assert!(
		has_query,
		"reusable-branch-health.yml's `gh api` call no longer asks for \
		 status=completed with per_page=30. Asking for the newest run of \
		 any status would report an in-progress run as a failure, and the \
		 pull request repairing the branch would block itself with a \
		 verdict nobody has finished reaching; per_page=1 would hand back \
		 whatever item happens to land at index 0, which the API does \
		 not guarantee is the newest run."
	);

	// And it must NOT ask for status=success alone, which would miss
	// every failing run that should be the loudest signal of all.
	let has_success_only = reusable.lines().any(|l| {
		let trimmed = l.trim_start();
		if trimmed.starts_with('#') {
			return false;
		}
		trimmed.contains("status=success")
	});
	assert!(
		!has_success_only,
		"reusable-branch-health.yml's `gh api` call asks for status=success \
		 on its own. That is the freshness watcher's shape and cannot \
		 catch this defect: a failing run is missing from its answer by \
		 construction. The whole point of this file is to read COMPLETED \
		 runs and decide the conclusion itself."
	);
}

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
/// `with.branch`. Anything stronger (`&& ...`) is fine in principle but
/// is rejected here so a future refactor that flips the conjunction or
/// appends `|| true` does not silently disarm the latching fix.
fn expected_health_if(branch: &str) -> String {
	format!("github.event_name != 'push' || github.ref_name != '{branch}'")
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
    if: github.event_name != 'push' || github.ref_name != 'develop'
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
    if: github.event_name != 'push' || github.ref_name != 'main' || true
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
