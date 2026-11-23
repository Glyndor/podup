//! The Rust lanes decide for themselves whether they have anything to do.
//!
//! `reusable-rust-ci.yml` asks `.github/scripts/checks/rust-ci-inputs.sh`
//! whether the range touches anything the lint, test, coverage and
//! extra-platform lanes read, and skips what it does not need to run.
//!
//! The names the ruleset requires are `rust / Format & lint`, `rust / Test`,
//! `rust / Coverage`, `rust / Doc warnings`, `rust / MSRV` and
//! `rust / Extra platforms`. A skipped job reports `skipped`, which a required
//! check accepts, so the jobs whose own name is required (`Test`, `Coverage`)
//! are skipped whole by an `if:` on the `classify` job's output. Two shapes
//! are never skipped that way:
//!
//! - `lint` also runs checks that must see a documentation-only change, so it
//!   skips only its heavy STEPS, behind the decision `classify` gives it.
//! - the macOS and Windows matrix is not required by name (`Test (${{
//!   matrix.os }})` moves with the matrix), `Extra platforms` is, so the
//!   matrix is skipped whole while the aggregate gate always runs.
//!
//! Properties asserted here:
//!
//! 1. Every heavy step of `lint` is behind the guard.
//! 2. Every guard fails closed. A decision that never arrived - no classifier
//!    in the base, a checkout without history, a crash, a `classify` job that
//!    itself failed - runs the lanes instead of skipping them. This matters
//!    more for job-level skips: a job whose `needs` failed is skipped by
//!    default, and a skipped job is a pass to a required check.
//! 3. The judge is read from the reviewed base commit, never from the pull
//!    request's own tree, so a pull request cannot rewrite the judge of its
//!    own diff.
//! 4. The aggregate gates keep running and fail on anything but success or
//!    skipped.
//!
//!    `rust / Gate` is the one that certifies the pipeline: it waits for every
//!    other job, runs on every outcome, and its verdict is exercised by
//!    `tests/scripts/shell/workflow_behaviour/rust-gate.test.sh`. The tests
//!    below hold its wiring: nothing it certifies is left out of its `needs`,
//!    and every job it certifies starts even when the plan failed.
//!
//! 5. The classifier lets files skip the Rust lanes that the Rust test
//!    `no_private_document_references` still reads (a shell test, the
//!    licence, a template). The skip is sound only while `lint` runs the same
//!    scan with no guard of its own, so that step has to stay and stay
//!    ungated.
//!
//! The policy itself is exercised by the shell suite in
//! `tests/scripts/shell/workflow_behaviour/rust-ci-inputs.test.sh`. This file
//! is about the wiring.

use std::fs;
use std::path::Path;

use serde_yaml::Value;

/// The jobs that keep running and skip only their heavy steps.
const STEP_GOVERNED: [(&str, &str); 1] = [("lint", "Format & lint")];

/// The jobs skipped whole when the classifier says nothing here is read, and
/// the check name each one reports.
///
/// The names are the ones the rulesets require; `tests/repository/workflows/
/// workflow_required_checks/shape.rs` holds the same list against the whole
/// tree, and this one covers only the jobs this change touches.
const JOB_GOVERNED: [(&str, &str); 3] = [
	("test", "Test"),
	("test-extra", "Test (${{ matrix.os }})"),
	("coverage", "Coverage"),
];

/// The guard every heavy step of a step-governed job carries.
const GUARD: &str = "needs.classify.outputs.run != 'false'";

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
	for (id, _) in STEP_GOVERNED {
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
	for (id, _) in STEP_GOVERNED {
		for step in steps(&job(&spec, id)) {
			let cond = if_text(&step);
			if !cond.contains("needs.classify.outputs.run") {
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
	// `classify` is the only job that judges, and it asks twice: which lanes
	// run, and whether the root build inputs changed.
	for (id, script) in [
		("classify", "rust-ci-inputs.sh"),
		("classify", "rust-build-inputs.sh"),
	] {
		let job = job(&spec, id);
		let asked: Vec<String> = steps(&job)
			.iter()
			.map(|s| run_text(s).to_string())
			.filter(|r| r.contains(script))
			.collect();
		assert_eq!(
			asked.len(),
			1,
			"{id} asks {script} {asked:?}; exactly one question per judge"
		);
		assert!(
			asked[0].contains(&format!(
				"git show \"$BASE_SHA:.github/scripts/checks/{script}\""
			)),
			"{id} does not read {script} from the reviewed base: {}",
			asked[0]
		);
		assert!(
			!asked[0].contains(&format!("bash .github/scripts/checks/{script}")),
			"{id} runs {script} from the working tree, so a pull request could \
			 rewrite the judge of its own diff: {}",
			asked[0]
		);
	}
	// No other job asks either judge: one plan, certified by the gate.
	for (id, j) in spec["jobs"].as_mapping().expect("jobs").iter() {
		let id = id.as_str().unwrap_or_default();
		if id == "classify" {
			continue;
		}
		for step in steps(j) {
			let run = run_text(&step);
			assert!(
				!run.contains("rust-ci-inputs.sh") && !run.contains("rust-build-inputs.sh"),
				"{id} judges the range itself ({:?}); the plan is `classify`'s alone, \
				 or the gate cannot certify what it skipped",
				step["name"]
			);
		}
	}
}

#[test]
fn the_jobs_skipped_whole_take_their_decision_from_classify_and_ask_nothing_themselves() {
	let spec = reusable();
	for (id, _) in JOB_GOVERNED {
		let job = job(&spec, id);
		assert_eq!(
			job["needs"].as_str(),
			Some("classify"),
			"{id} does not wait for the classifier"
		);
		for step in steps(&job) {
			assert!(
				!run_text(&step).contains("rust-ci-inputs.sh")
					&& !if_text(&step).contains("steps.decide"),
				"{id} still asks the classifier itself ({:?}); the answer comes \
				 from `classify`",
				step["name"]
			);
		}
	}
}

#[test]
fn the_job_guard_runs_the_job_when_the_classifier_did_not_answer() {
	let spec = reusable();
	for (id, _) in JOB_GOVERNED {
		let cond = job(&spec, id)["if"]
			.as_str()
			.unwrap_or_default()
			.to_string();
		for needle in [
			"needs.classify.outputs.run != 'false'",
			"needs.classify.result != 'success'",
			"!cancelled()",
		] {
			assert!(
				cond.contains(needle),
				"{id} is guarded by {cond:?}, which lacks {needle:?}. A classifier \
				 that failed would skip the job, a skipped job is a pass to a \
				 required check, and a broken checkout would turn the suite green."
			);
		}
		assert!(
			!cond.contains("always()") && !cond.contains("== 'false'"),
			"{id} is guarded by {cond:?}; the guard must skip only on an explicit \
			 `false`"
		);
	}
}

#[test]
fn classify_exposes_the_decision_and_the_job_that_must_not_be_skipped_stays_whole() {
	let spec = reusable();
	assert_eq!(
		job(&spec, "classify")["outputs"]["run"].as_str(),
		Some("${{ steps.decide.outputs.run }}"),
		"classify no longer exposes the answer the other jobs read"
	);
	assert_eq!(
		job(&spec, "classify")["outputs"]["build"].as_str(),
		Some("${{ steps.build.outputs.build }}"),
		"classify no longer exposes the build answer `msrv`, `doc` and the gate read"
	);
	for id in ["lint", "msrv-gate"] {
		let cond = job(&spec, id)["if"]
			.as_str()
			.unwrap_or_default()
			.to_string();
		assert!(
			!cond.contains("outputs."),
			"{id} reads a classifier answer at job level. `lint` runs checks a \
			 documentation-only change must see, and the MSRV gate is the name the \
			 ruleset requires: {cond:?}"
		);
	}
}

#[test]
fn the_governed_jobs_check_out_the_history_the_classifier_reads() {
	let spec = reusable();
	let checkout = steps(&job(&spec, "classify"))
		.into_iter()
		.find(|s| {
			s["uses"]
				.as_str()
				.unwrap_or_default()
				.starts_with("actions/checkout")
		})
		.unwrap_or_else(|| panic!("classify checks the repository out"));
	assert_eq!(
		checkout["with"]["fetch-depth"].as_u64(),
		Some(0),
		"classify does not fetch the history, so the classifier cannot read the \
		 base commit or diff the range"
	);
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
	for (id, name) in STEP_GOVERNED.into_iter().chain(JOB_GOVERNED) {
		assert_eq!(
			job(&spec, id)["name"].as_str(),
			Some(name),
			"{id} no longer reports the name it reported before"
		);
	}
}

/// `msrv` and `doc` are skipped whole on the plan's build answer, and only on
/// an explicit `false`.
#[test]
fn msrv_and_doc_skip_only_on_an_explicit_false_from_the_plan() {
	let spec = reusable();
	for id in ["msrv", "doc"] {
		let job = job(&spec, id);
		assert_eq!(
			job["needs"].as_str(),
			Some("classify"),
			"{id} does not wait for the plan"
		);
		let cond = job["if"].as_str().unwrap_or_default();
		assert!(
			cond.contains("!cancelled()")
				&& cond.contains("needs.classify.result != 'success'")
				&& cond.contains("needs.classify.outputs.build != 'false'"),
			"{id} is guarded by {cond:?}; it must start when the plan failed and \
			 skip only on an explicit `false`"
		);
		assert!(
			!cond.contains("== 'true'") && !cond.contains("always()"),
			"{id} is guarded by {cond:?}"
		);
	}
}

/// The gate waits for every other job, runs on every outcome, and every job it
/// certifies starts even when the plan failed.
#[test]
fn the_gate_waits_for_every_job_and_every_suite_starts_when_the_plan_failed() {
	let spec = reusable();
	let gate = job(&spec, "gate");
	assert!(
		gate["name"]
			.as_str()
			.unwrap_or_default()
			.starts_with("Gate"),
		"the gate keeps the name the ruleset requires, with the run kind after it"
	);
	assert_eq!(
		gate["if"].as_str(),
		Some("always()"),
		"the gate must run whatever the suites did, or a skip leaves it unreported"
	);
	let needs: Vec<&str> = gate["needs"]
		.as_sequence()
		.expect("the gate lists its needs")
		.iter()
		.filter_map(Value::as_str)
		.collect();
	// The two older gates are the names the ruleset still requires until it
	// moves to `Gate`; they read the matrix and the MSRV job themselves.
	for (id, _) in spec["jobs"].as_mapping().expect("jobs").iter() {
		let id = id.as_str().unwrap_or_default();
		if id == "gate" || id == "extra-platforms-gate" || id == "msrv-gate" {
			continue;
		}
		assert!(
			needs.contains(&id),
			"the gate does not wait for `{id}`, so a result it never reads can \
			 leave the pull request green"
		);
	}
	for id in ["lint", "test", "test-extra", "coverage", "msrv", "doc"] {
		let cond = job(&spec, id)["if"]
			.as_str()
			.unwrap_or_default()
			.to_string();
		assert!(
			cond.contains("!cancelled()"),
			"{id} is guarded by {cond:?}. Without `!cancelled()` it is skipped \
			 when the plan failed, and a job skipped for that reason is the \
			 fail-open the gate exists to catch"
		);
	}
}

/// The gates that stand in for the extra-platform matrix and for the MSRV job
/// accept a skipped dependency only when the caller switched it off or a plan
/// that succeeded said so out loud, so a matrix that never ran for any other
/// reason leaves the required name red. They read the plan themselves.
#[test]
fn the_older_gates_certify_a_skip_against_the_plan_and_reject_the_rest() {
	let spec = reusable();
	for (id, dep) in [
		("extra-platforms-gate", "test-extra"),
		("msrv-gate", "msrv"),
	] {
		let gate = job(&spec, id);
		let needs: Vec<&str> = gate["needs"]
			.as_sequence()
			.expect("the gate lists its needs")
			.iter()
			.filter_map(Value::as_str)
			.collect();
		assert_eq!(
			needs,
			["classify", dep],
			"{id} must read the plan and {dep}"
		);
		assert_eq!(
			gate["if"].as_str(),
			Some("always()"),
			"{id} must run whatever happened, or a skipped matrix leaves the \
			 required check missing"
		);
		let body: String = steps(&gate)
			.iter()
			.map(run_text)
			.collect::<Vec<_>>()
			.join("\n");
		assert!(
			body.contains("[ \"$PLAN\" = success ]"),
			"{id} accepts a skip without asking whether the plan succeeded: {body}"
		);
		assert!(
			body.contains("= false ]") && body.contains("exit 1"),
			"{id} no longer fails on a failed or cancelled dependency: {body}"
		);
	}
}

/// The classifier skips files that `no_private_document_references` reads.
/// The step that repeats the scan outside the Rust lanes must exist and must
/// carry no condition, or a shell-only pull request is checked by nothing.
#[test]
fn the_private_document_scan_runs_in_lint_whatever_the_classifier_says() {
	let spec = reusable();
	let scans: Vec<Value> = steps(&job(&spec, "lint"))
		.into_iter()
		.filter(|s| run_text(s).contains("check-private-document-references.sh"))
		.collect();
	assert_eq!(
		scans.len(),
		1,
		"lint must run the private-document scan exactly once, found {}. The \
		 classifier lets a shell test, the licence or a template skip the Rust \
		 lanes, and this scan is the only thing left that reads them.",
		scans.len()
	);
	assert!(
		scans[0].get("if").is_none(),
		"the private-document scan is guarded by {:?}; guarded by the \
		 classifier it stops reading exactly the files the classifier skips",
		scans[0]["if"]
	);
	assert!(
		scans[0].get("continue-on-error").is_none(),
		"the private-document scan must be able to fail the job"
	);
}
