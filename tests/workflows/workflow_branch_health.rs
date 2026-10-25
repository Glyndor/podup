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
//!
//! This crate root holds the readers and `with:` parsers every scenario
//! file inherits through `use super::*`, plus one `mod` line per scenario.
//! The rows themselves live in the files declared below.

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
/// Same shape as `tests/workflows/workflow_toolchain_pin/deb_pair.rs`'s scoped
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

// Property 1, with the planted negative control for its parser.
#[path = "workflow_branch_health/branch_coverage.rs"]
mod branch_coverage;

// The reusable checks out the tree before running the script it ships.
#[path = "workflow_branch_health/reusable_steps.rs"]
mod reusable_steps;

// Property 2: the API query the conclusion script makes.
#[path = "workflow_branch_health/script_query.rs"]
mod script_query;

// Property 3: the check is advisory and both files say so.
#[path = "workflow_branch_health/advisory_status.rs"]
mod advisory_status;

// Each caller carries the `if:` that keeps it out of its own push run.
#[path = "workflow_branch_health/self_branching_if.rs"]
mod self_branching_if;

// The `HEAD_SHA:` line gates the commit to the branch under test.
#[path = "workflow_branch_health/head_sha_expression.rs"]
mod head_sha_expression;
