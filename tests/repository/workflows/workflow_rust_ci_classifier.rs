//! The Rust lanes decide for themselves whether they have anything to do.
//!
//! `reusable-rust-ci.yml` asks `.github/scripts/checks/rust-ci-inputs.sh`
//! whether the range touches anything the lint, test, coverage and
//! extra-platform lanes read, and skips the heavy steps when it does not.
//!
//! The decision happens INSIDE each job, because the names the ruleset
//! requires are reported by those jobs: `rust / Format & lint`, `rust / Test`,
//! `rust / Coverage`, `rust / Doc warnings`, `rust / MSRV` and
//! `rust / Extra platforms`. A workflow that never starts leaves its check
//! pending and blocks the pull request for good, so the classifier skips
//! STEPS and never jobs.
//!
//! Three properties make the skip safe, and all three are asserted here:
//!
//! 1. Every step that builds something is behind the guard. A heavy step
//!    outside it would run on a pull request the classifier judged
//!    irrelevant, and the classification would be a claim instead of a fact.
//! 2. The guard fails closed. It reads `run != 'false'`, so a decision that
//!    never arrived - no classifier in the base, a checkout without history,
//!    a crash - runs the lanes instead of skipping them.
//! 3. The judge is read from the reviewed base commit, never from the pull
//!    request's own tree, so a pull request cannot rewrite the judge of its
//!    own diff.
//!
//! The policy itself is exercised by the shell suite in
//! `tests/scripts/shell/workflow_behaviour/rust-ci-inputs.test.sh`. This file
//! is about the wiring.

use std::fs;
use std::path::Path;

use serde_yaml::Value;

/// The jobs the classifier governs, and the check name each one reports.
///
/// The names are the ones the rulesets require; `tests/repository/workflows/
/// workflow_required_checks/shape.rs` holds the same list against the whole
/// tree, and this one covers only the jobs this change touches.
const GOVERNED: [(&str, &str); 4] = [
	("lint", "Format & lint"),
	("test", "Test"),
	("test-extra", "Test (${{ matrix.os }})"),
	("coverage", "Coverage"),
];

/// The guard every heavy step carries.
const GUARD: &str = "steps.decide.outputs.run != 'false'";

fn reusable() -> Value {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/reusable-rust-ci.yml");
	let text =
		fs::read_to_string(&path).unwrap_or_else(|e| panic!("the reusable is readable: {e}"));
	serde_yaml::from_str(&text).expect("the reusable parses as YAML")
}

fn job(spec: &Value, id: &str) -> Value {
	spec["jobs"][id].clone()
}

fn steps(job: &Value) -> Vec<Value> {
	job["steps"].as_sequence().cloned().unwrap_or_default()
}

fn run_text(step: &Value) -> &str {
	step["run"].as_str().unwrap_or_default()
}

fn if_text(step: &Value) -> &str {
	step["if"].as_str().unwrap_or_default()
}

/// A step that costs runner time: it compiles, installs a toolchain or starts
/// a service.
fn is_heavy(step: &Value) -> bool {
	let run = run_text(step);
	run.contains("cargo ") || run.contains("rustup ") || run.contains("systemctl ")
}

#[test]
fn every_step_that_builds_something_is_behind_the_guard() {
	let spec = reusable();
	for (id, _) in GOVERNED {
		let mut heavy = 0;
		let mut guarded = 0;
		for step in steps(&job(&spec, id)) {
			if !is_heavy(&step) {
				continue;
			}
			heavy += 1;
			if if_text(&step).contains(GUARD) {
				guarded += 1;
			}
		}
		assert!(
			heavy > 0,
			"{id} has no heavy step, so this case proves nothing"
		);
		assert_eq!(
			heavy, guarded,
			"{id} runs {heavy} heavy steps and guards {guarded} of them. An \
			 unguarded one runs on a pull request the classifier judged \
			 irrelevant, and the classification becomes a claim."
		);
	}
}

#[test]
fn the_guard_runs_the_lanes_when_the_decision_never_arrived() {
	let spec = reusable();
	for (id, _) in GOVERNED {
		for step in steps(&job(&spec, id)) {
			let cond = if_text(&step);
			if !cond.contains("steps.decide.outputs.run") {
				continue;
			}
			assert!(
				cond.contains("!= 'false'"),
				"{id} guards a step with {cond:?}. That skips the lanes when the \
				 decision is missing, and a missing decision has to run them."
			);
		}
	}
}

#[test]
fn the_judge_is_read_from_the_base_and_never_from_the_working_tree() {
	let spec = reusable();
	for (id, _) in GOVERNED {
		let job = job(&spec, id);
		let asked: Vec<String> = steps(&job)
			.iter()
			.map(|s| run_text(s).to_string())
			.filter(|r| r.contains("rust-ci-inputs.sh"))
			.collect();
		assert_eq!(
			asked.len(),
			1,
			"{id} asks the classifier {asked:?}; exactly one question per job"
		);
		assert!(
			asked[0].contains("git show \"$BASE_SHA:.github/scripts/checks/rust-ci-inputs.sh\""),
			"{id} does not read the judge from the reviewed base: {}",
			asked[0]
		);
		assert!(
			!asked[0].contains("bash .github/scripts/checks/rust-ci-inputs.sh"),
			"{id} runs the judge from the working tree, so a pull request could \
			 rewrite the judge of its own diff: {}",
			asked[0]
		);
	}
}

#[test]
fn the_governed_jobs_check_out_the_history_the_classifier_reads() {
	let spec = reusable();
	for (id, _) in GOVERNED {
		let checkout = steps(&job(&spec, id))
			.into_iter()
			.find(|s| {
				s["uses"]
					.as_str()
					.unwrap_or_default()
					.starts_with("actions/checkout")
			})
			.unwrap_or_else(|| panic!("{id} checks the repository out"));
		assert_eq!(
			checkout["with"]["fetch-depth"].as_u64(),
			Some(0),
			"{id} does not fetch the history, so the classifier cannot read the \
			 base commit or diff the range"
		);
	}
}

#[test]
fn the_range_comes_from_the_event_and_covers_the_whole_merge_group() {
	let spec = reusable();
	let env = &spec["env"];
	assert_eq!(env["EVENT"].as_str(), Some("${{ github.event_name }}"));
	for (key, pull, group) in [
		(
			"BASE_SHA",
			"github.event.pull_request.base.sha",
			"github.event.merge_group.base_sha",
		),
		(
			"HEAD_SHA",
			"github.event.pull_request.head.sha",
			"github.event.merge_group.head_sha",
		),
	] {
		let value = env[key].as_str().unwrap_or_default();
		assert!(
			value.contains(pull),
			"{key} does not name the pull request's {pull}"
		);
		assert!(
			value.contains(group),
			"{key} does not name the merge group's {group}; a merge group has to \
			 be judged on the whole range the queue built"
		);
	}
}

#[test]
fn the_check_names_the_ruleset_requires_are_unchanged() {
	let spec = reusable();
	for (id, name) in [
		("lint", "Format & lint"),
		("test", "Test"),
		("coverage", "Coverage"),
		("doc", "Doc warnings"),
		("msrv-gate", "MSRV"),
		("extra-platforms-gate", "Extra platforms"),
	] {
		assert_eq!(
			job(&spec, id)["name"].as_str(),
			Some(name),
			"{id} no longer reports the name the ruleset requires"
		);
	}
	for (id, name) in GOVERNED {
		assert_eq!(
			job(&spec, id)["name"].as_str(),
			Some(name),
			"{id} no longer reports the name it reported before"
		);
	}
}

/// The gate that stands in for the extra-platform matrix treats a skipped
/// dependency as a pass, so a matrix leg that never ran still leaves
/// `rust / Extra platforms` reported.
#[test]
fn the_extra_platform_gate_accepts_a_matrix_that_did_not_run() {
	let spec = reusable();
	let body: String = steps(&job(&spec, "extra-platforms-gate"))
		.iter()
		.map(run_text)
		.collect::<Vec<_>>()
		.join("\n");
	assert!(
		body.contains("success | skipped)"),
		"the extra-platform gate no longer accepts a skipped matrix leg, so \
		 `rust / Extra platforms` stops being reported when the matrix did \
		 not run: {body}"
	);
}
