//! The workflow lint decides for itself whether it has anything to do.
//!
//! `reusable-workflow-lint.yml` asks
//! `.github/scripts/checks/workflow-lint-inputs.sh` whether the range touches
//! anything the lint reads, and skips the heavy steps when it does not.
//! Installing Go, installing actionlint and downloading a pinned shellcheck
//! is a minute of runner time on every pull request, and a pull request
//! that touches no workflow pays all of it for nothing.
//!
//! The decision happens INSIDE the job, because the name the ruleset
//! requires is reported by that job: `workflow-lint / workflow-lint`. A
//! workflow that never starts leaves its check pending and blocks the pull
//! request for good, so the classifier skips STEPS and never jobs.
//!
//! Three properties make the skip safe, and all three are asserted here:
//!
//! 1. Every step that installs a tool or reads a workflow is behind the
//!    guard. A step outside it would run on a pull request the classifier
//!    judged irrelevant, and the classification would be a claim instead of
//!    a fact.
//! 2. The guard fails closed. It reads `run != 'false'`, so a decision that
//!    never arrived - no judge in the base, a checkout without history, a
//!    crash - runs the lint instead of skipping it.
//! 3. The judge is read from the reviewed base commit, never from the pull
//!    request's own tree, so a pull request cannot rewrite the judge of its
//!    own diff.
//!
//! Two more are asserted here because they are the property the whole
//! change rests on: the policy names every file the lint reads, and the
//! scripts the lint runs live where the policy looks.
//!
//! The policy itself is exercised by the shell suite in
//! `tests/scripts/shell/workflow_behaviour/workflow-lint-inputs.test.sh`.
//! This file is about the wiring.

use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml::Value;

/// The reusable that holds the lint.
const REUSABLE: &str = ".github/workflows/reusable-workflow-lint.yml";
/// The workflow that calls it.
const CALLER: &str = ".github/workflows/workflow-lint.yml";
/// The judge, read from the base commit.
const JUDGE: &str = ".github/scripts/checks/workflow-lint-inputs.sh";
/// The guard every step that installs a tool or reads a workflow carries.
const GUARD: &str = "steps.decide.outputs.run != 'false'";

/// The check name the ruleset requires, and the two halves it is made of.
///
/// The caller names its job `workflow-lint` and the reusable's job is named
/// `workflow-lint`, so GitHub reports `workflow-lint / workflow-lint`. Both
/// halves are load-bearing: a renamed job leaves the ruleset waiting for a
/// check nothing emits.
const CHECK_NAME: &str = "workflow-lint / workflow-lint";

fn root() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(path: &str) -> String {
	let path = root().join(path);
	fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
}

fn doc(path: &str) -> Value {
	serde_yaml::from_str(&read(path)).unwrap_or_else(|e| panic!("{path} parses as YAML: {e}"))
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

/// A step that may run whatever the judge says: the checkout, which has to
/// happen before the judge can be read at all, and the judge itself.
fn is_exempt(step: &Value) -> bool {
	let uses = step["uses"].as_str().unwrap_or_default();
	uses.starts_with("actions/checkout") || run_text(step).contains(JUDGE)
}

#[test]
fn every_step_that_installs_or_reads_is_behind_the_guard() {
	let spec = doc(REUSABLE);
	let job = spec["jobs"]["workflow-lint"].clone();
	let mut guarded = 0;
	let mut bare = 0;
	for step in steps(&job) {
		if is_exempt(&step) {
			continue;
		}
		if if_text(&step).contains(GUARD) {
			guarded += 1;
		} else {
			bare += 1;
		}
	}
	assert!(
		guarded > 0,
		"no step is behind the guard, so this case proves nothing"
	);
	assert_eq!(
		bare, 0,
		"{bare} steps run whatever the judge says. A step that installs a tool \
		 or reads a workflow outside the guard runs on a pull request the \
		 judge judged irrelevant, and the classification becomes a claim."
	);
}

#[test]
fn the_guard_runs_the_lint_when_the_decision_never_arrived() {
	let spec = doc(REUSABLE);
	for step in steps(&spec["jobs"]["workflow-lint"].clone()) {
		let cond = if_text(&step);
		if !cond.contains("steps.decide.outputs.run") {
			continue;
		}
		assert!(
			cond.contains("!= 'false'"),
			"a step is guarded with {cond:?}. That skips the lint when the \
			 decision is missing, and a missing decision has to run it."
		);
	}
}

#[test]
fn the_judge_is_read_from_the_base_and_never_from_the_working_tree() {
	let spec = doc(REUSABLE);
	let asked: Vec<String> = steps(&spec["jobs"]["workflow-lint"].clone())
		.iter()
		.map(|s| run_text(s).to_string())
		.filter(|r| r.contains(JUDGE))
		.collect();
	assert_eq!(
		asked.len(),
		1,
		"the job asks the judge {asked:?}; exactly one question per job"
	);
	assert!(
		asked[0].contains(&format!("git show \"$BASE_SHA:{JUDGE}\"")),
		"the job does not read the judge from the reviewed base: {}",
		asked[0]
	);
	assert!(
		!asked[0].contains(&format!("bash {JUDGE}")),
		"the job runs the judge from the working tree, so a pull request could \
		 rewrite the judge of its own diff: {}",
		asked[0]
	);
}

#[test]
fn the_job_checks_out_the_history_the_judge_reads() {
	let spec = doc(REUSABLE);
	let checkout = steps(&spec["jobs"]["workflow-lint"].clone())
		.into_iter()
		.find(|s| {
			s["uses"]
				.as_str()
				.unwrap_or_default()
				.starts_with("actions/checkout")
		})
		.unwrap_or_else(|| panic!("the job checks the repository out"));
	assert_eq!(
		checkout["with"]["fetch-depth"].as_u64(),
		Some(0),
		"the job does not fetch the history, so the judge cannot read the base \
		 commit or diff the range"
	);
}

#[test]
fn the_range_comes_from_the_event_and_covers_the_whole_merge_group() {
	let spec = doc(REUSABLE);
	let env = &spec["jobs"]["workflow-lint"]["env"];
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
			"{key} does not name the merge group's {group}; a merge group has \
			 to be judged on the whole range the queue built"
		);
	}
}

#[test]
fn the_check_name_the_ruleset_requires_is_unchanged() {
	let reusable = doc(REUSABLE);
	assert_eq!(
		reusable["jobs"]["workflow-lint"]["name"].as_str(),
		Some("workflow-lint"),
		"the reusable's job no longer reports the name it reported before"
	);
	let caller = doc(CALLER);
	let names: Vec<&str> = caller["jobs"]
		.as_mapping()
		.expect("the caller declares jobs")
		.keys()
		.filter_map(|k| k.as_str())
		.collect();
	assert!(
		names.contains(&"workflow-lint"),
		"the caller no longer names a job `workflow-lint`, so the check it \
		 reports is no longer `{CHECK_NAME}`"
	);
}

/// The policy is the reason the skip is safe, and the lint's verdict is a
/// function of the files it reads. A file the policy does not name is a
/// file whose change cannot reach the lint, and the policy has to name
/// every one of them.
#[test]
fn the_policy_names_every_file_the_lint_reads() {
	let judge = read(JUDGE);
	for needle in [
		".github/workflows/",
		".github/scripts/",
		".github/actionlint.yaml",
		"tests/repository/workflows/",
		"tests/scripts/shell/workflow_",
		"Cargo.toml",
	] {
		assert!(
			judge.contains(needle),
			"the judge does not name {needle}, so a change to it cannot reach \
			 the lint even though the lint reads it"
		);
	}
}

/// The scripts the lint runs are the other half of its inputs, and the
/// policy reaches them through the directory they live in. A script outside
/// that directory would be a script the policy cannot see.
#[test]
fn the_scripts_the_lint_runs_live_where_the_policy_looks() {
	let body = read(REUSABLE);
	let mut scripts = 0;
	for line in body.lines() {
		let line = line.trim();
		let Some(rest) = line.strip_prefix("python3 .github/scripts/") else {
			continue;
		};
		let path = format!(".github/scripts/{rest}");
		assert!(
			path.starts_with(".github/scripts/checks/"),
			"{path} runs outside .github/scripts/checks/, and the judge names \
			 that directory rather than the tree"
		);
		assert!(
			root().join(&path).exists(),
			"{path} is invoked by the lint and does not exist"
		);
		scripts += 1;
	}
	assert!(
		scripts > 0,
		"the lint runs no script, so this case proves nothing"
	);
}
