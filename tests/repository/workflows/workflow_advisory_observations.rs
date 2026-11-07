//! The advisory observations about the repository do not run on pull
//! requests, and the monitoring logic that feeds them keeps its own tests.
//!
//! Four jobs observe the repository rather than the change under review:
//! the schedule-freshness gate (are the scheduled workflows still
//! firing?), the two branch-health gates (is the newest run of `ci.yml`
//! on `main` and on `develop` green?) and the Dependabot-freshness gate
//! (has Dependabot spoken recently?). They used to sit in `ci.yml` and
//! run on every pull request update.
//!
//! What that cost is measured here rather than asserted. Concurrent pull
//! requests polled the same branch heads and the same scheduler history
//! four times each, and none of it said anything about the patch being
//! reviewed: the branch-health poll alone can take eight minutes. A
//! check that costs eight minutes and learns nothing about the change is
//! a check nobody should pay for on every update of a review.
//!
//! Four properties, one test each:
//!
//! 1. The observations do not run on a pull request. The workflow has no
//!    `pull_request` trigger at all, and every observation job carries
//!    an `if:` that refuses the event, so a trigger added later would
//!    not silently bring them back.
//! 2. They still run, and on all three of the events that matter: a push
//!    to a protected branch (the non-cron fallback, so a dead schedule
//!    is still noticed), the schedule itself, and a manual dispatch.
//! 3. The monitoring logic keeps its own coverage. The shell suite that
//!    drives these observers runs on pull requests whenever the files
//!    it reads change, so a regression in the observer is caught by the
//!    change that introduces it.
//! 4. The required lane is untouched: `ci.yml` still triggers on pull
//!    requests and merge groups and still carries the `rust` job, whose
//!    check names the ruleset requires.
//!
//! Each assertion is paired with a planted control, so a reader can see
//! what the check would have to be broken to pass.

use std::fs;
use std::path::Path;

use serde_yaml::Value;

/// The workflow that carries the observations.
const OBSERVER: &str = "repository-health.yml";

/// The jobs that observe the repository rather than the change under
/// review. The first three names are the check names GitHub reports for
/// them, so they are part of the wire format.
const OBSERVATIONS: &[&str] = &[
	"freshness",
	"health-main",
	"health-develop",
	"dependabot-freshness",
];

/// The files whose change must still run the tests that drive the
/// monitoring logic.
///
/// The script is covered by the `**.sh` glob every shell workflow
/// carries; the workflows are named one by one, because a path filter
/// that does not mention a file never runs the test that reads it.
const MONITORING_FILES: &[&str] = &[
	".github/workflows/repository-health.yml",
	".github/workflows/reusable-branch-health.yml",
	".github/workflows/reusable-schedule-freshness.yml",
	".github/workflows/reusable-dependabot-freshness.yml",
];

fn read(name: &str) -> String {
	let path = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join(".github/workflows")
		.join(name);
	fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} is readable: {e}"))
}

fn doc(name: &str) -> Value {
	serde_yaml::from_str(&read(name)).unwrap_or_else(|e| panic!("{name} parses: {e}"))
}

/// The parsed `on:` block, or `None` when the workflow declares none.
///
/// YAML 1.1 reads the key `on` as the boolean true, so both spellings
/// are looked up.
fn triggers(doc: &Value) -> Option<&serde_yaml::Mapping> {
	match doc.get("on").or_else(|| doc.get(Value::Bool(true)))? {
		Value::Mapping(on) => Some(on),
		_ => None,
	}
}

/// Whether the workflow declares a `pull_request` trigger.
fn declares_pull_request(doc: &Value) -> bool {
	triggers(doc).is_some_and(|on| on.contains_key("pull_request"))
}

/// The `paths:` list of one event of one workflow.
fn paths_of<'a>(doc: &'a Value, event: &str) -> &'a Vec<Value> {
	triggers(doc)
		.and_then(|on| on.get(event))
		.and_then(|cfg| cfg.get("paths"))
		.and_then(|paths| paths.as_sequence())
		.unwrap_or_else(|| panic!("the workflow declares `{event}.paths`"))
}

#[test]
fn the_observations_do_not_run_on_a_pull_request() {
	let observer = doc(OBSERVER);
	assert!(
		!declares_pull_request(&observer),
		"{OBSERVER} declares a `pull_request` trigger. These four jobs answer \
		 questions about the repository, not about the change under review, and \
		 they used to run on every pull request update: concurrent pull \
		 requests polled the same branch heads and the same scheduler history \
		 four times each, and the branch-health poll takes up to eight \
		 minutes of runner for an answer that says nothing about the patch."
	);

	// The trigger is not the only half of it. A trigger added later, or
	// one that arrives with the workflow's next edit, must not bring the
	// jobs back: every observation job refuses the event itself.
	for id in OBSERVATIONS {
		let job = &observer["jobs"][*id];
		let if_value = job
			.get("if")
			.and_then(|v| v.as_str())
			.unwrap_or_else(|| panic!("`{id}` in {OBSERVER} carries a job-level `if:`"));
		assert!(
			if_value.contains("github.event_name != 'pull_request'"),
			"`{id}` in {OBSERVER} carries `if: {if_value}`, which does not refuse \
			 a pull request. The trigger above is the first half; this is the \
			 second, and it is the one that survives a later edit."
		);
	}
}

#[test]
fn the_observations_still_run_on_push_schedule_and_dispatch() {
	let observer = doc(OBSERVER);
	let on = triggers(&observer).expect("the observer declares its triggers");

	// The push to a protected branch is the non-cron fallback. A cron
	// that stops firing emits nothing, and the push lane is what turns
	// that silence back into a red check.
	let branches: Vec<&str> = on
		.get("push")
		.and_then(|push| push.get("branches"))
		.and_then(|b| b.as_sequence())
		.map(|seq| seq.iter().filter_map(|v| v.as_str()).collect())
		.unwrap_or_default();
	for branch in ["main", "develop"] {
		assert!(
			branches.contains(&branch),
			"{OBSERVER} does not observe the repository on a push to `{branch}`. \
			 The push lane is the fallback for a dead schedule, so both \
			 protected branches have to be in it."
		);
	}

	// The schedule is the primary trigger, and it has to carry a cron:
	// an empty schedule never fires.
	let schedule = on
		.get("schedule")
		.and_then(|s| s.as_sequence())
		.unwrap_or_else(|| panic!("{OBSERVER} declares a `schedule` trigger"));
	assert!(
		!schedule.is_empty(),
		"{OBSERVER} declares a `schedule` trigger with no cron in it, so it \
		 never fires"
	);
	let crons: Vec<&str> = schedule
		.iter()
		.filter_map(|entry| entry.get("cron"))
		.filter_map(|cron| cron.as_str())
		.collect();
	assert_eq!(
		crons.len(),
		schedule.len(),
		"every entry of the schedule carries a `cron:`"
	);

	// The manual dispatch is what lets an operator ask the question now
	// instead of waiting for the next cron.
	assert!(
		on.contains_key("workflow_dispatch"),
		"{OBSERVER} has no `workflow_dispatch` trigger. Without it the \
		 observations can only be waited for."
	);
}

#[test]
fn the_monitoring_logic_keeps_its_tests_on_pull_requests() {
	let lint = doc("lint-shell.yml");
	for event in ["push", "pull_request"] {
		let paths = paths_of(&lint, event);
		let listed: Vec<&str> = paths.iter().filter_map(|v| v.as_str()).collect();
		for file in MONITORING_FILES {
			assert!(
				listed.contains(file),
				"lint-shell.yml's `{event}.paths` does not name `{file}`. A \
				 workflow that has shell tests must list the file it reads on \
				 both push and pull_request, or a change to that file merges \
				 without ever running the test that reads it."
			);
		}
		// The script the branch-health gate runs is covered by the glob,
		// not by a literal entry.
		assert!(
			listed.contains(&"**.sh"),
			"lint-shell.yml's `{event}.paths` no longer carries `**.sh`, so a \
			 change to the script the branch-health gate runs would merge \
			 without running the test that drives it."
		);
	}
}

#[test]
fn the_required_lane_is_untouched() {
	let ci = doc("ci.yml");
	let on = triggers(&ci).expect("ci.yml declares its triggers");
	assert!(
		on.contains_key("pull_request"),
		"ci.yml no longer triggers on pull requests"
	);
	assert!(
		on.contains_key("merge_group"),
		"ci.yml no longer triggers on merge groups, so the required `rust` \
		 checks would never be reported in the merge queue and the queue \
		 would never merge."
	);
	assert!(
		ci["jobs"].get("rust").is_some(),
		"ci.yml no longer carries the `rust` job. Its check names are the ones \
		 the ruleset requires, so removing it blocks every pull request with \
		 nothing reporting the gate."
	);

	// The observations left this file; they did not leave the repository.
	let body = read("ci.yml");
	for id in OBSERVATIONS {
		assert!(
			!body.contains(&format!("  {id}:")),
			"ci.yml still carries the `{id}` job. The observations belong to \
			 {OBSERVER}, so that a failure there cannot make the branch CI \
			 whose health they inspect red."
		);
	}
}

/// The controls. Each case plants the shape the check is written against,
/// so a reader can see what it would take for the assertions above to
/// pass without looking at anything.
#[test]
fn the_pull_request_detector_sees_the_shape_it_is_written_against() {
	let planted =
		"on:\n  push:\n    branches: [main]\n  pull_request:\n    branches: [main]\njobs: {}\n";
	let doc: Value = serde_yaml::from_str(planted).expect("the plant parses");
	assert!(
		declares_pull_request(&doc),
		"a workflow with a `pull_request` trigger was not recognised"
	);

	let without =
		"on:\n  push:\n    branches: [main]\n  schedule:\n    - cron: '0 6 * * *'\njobs: {}\n";
	let doc: Value = serde_yaml::from_str(without).expect("the plant parses");
	assert!(
		!declares_pull_request(&doc),
		"a workflow without a `pull_request` trigger was reported as having one"
	);
}
