//! The Debian lane decides for itself whether it has anything to do.
//!
//! `reusable-rust-debian.yml` asks `.github/scripts/checks/debian-ci-inputs.sh`
//! whether the range touches anything the package build reads, and skips the
//! heavy steps when it does not. A shell test script under `tests/scripts`
//! cannot reach the .deb, and a pull request that changes only one used to
//! pay a full package build for it.
//!
//! The decision happens INSIDE each job, because the names the ruleset
//! requires are reported by those jobs: `debian / dpkg-buildpackage` and
//! `debian / build (vendored / offline)`. A workflow that never starts leaves
//! its check pending and blocks the pull request for good, so the classifier
//! skips STEPS and never jobs.
//!
//! Four properties make the skip safe, and all four are asserted here:
//!
//! 1. Every step that builds something is behind the guard. A heavy step
//!    outside it would run on a pull request the classifier judged
//!    irrelevant, and the classification would be a claim instead of a fact.
//! 2. The guard fails closed. It reads `run != 'false'`, so a decision that
//!    never arrived - no classifier in the base, a checkout without history,
//!    a crash - builds the package instead of skipping it.
//! 3. The judge is read from the reviewed base commit, never from the pull
//!    request's own tree, so a pull request cannot rewrite the judge of its
//!    own diff.
//! 4. The caller still watches the whole test tree. The classifier decides
//!    whether to build, it never decides whether to run: #1295 rode twenty
//!    pull requests because the lane that compiles with debian/rules' feature
//!    set did not run at all.
//!
//! The policy itself is exercised by the shell suite in
//! `tests/scripts/shell/workflow_behaviour/debian-ci-inputs.test.sh`. This file
//! is about the wiring.

use std::fs;
use std::path::Path;

use serde_yaml::Value;

/// The jobs the classifier governs, and the check name each one reports.
///
/// The names are the ones the rulesets require; `tests/repository/workflows/
/// workflow_required_checks/shape.rs` holds the same list against the whole
/// tree, and this one covers only the jobs this change touches.
const GOVERNED: [(&str, &str); 2] = [
	("build", "dpkg-buildpackage"),
	("vendored", "build (vendored / offline)"),
];

/// The guard every heavy step carries.
const GUARD: &str = "steps.decide.outputs.run != 'false'";

/// The one heavy step that cannot be behind the guard: it installs the git
/// the classifier needs, because the classifier reads the judge from the
/// reviewed base commit and diffs the whole range.
const PREREQUISITE: &str = "Install git for the classifier";

fn reusable() -> Value {
	let path =
		Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/reusable-rust-debian.yml");
	let text =
		fs::read_to_string(&path).unwrap_or_else(|e| panic!("the reusable is readable: {e}"));
	serde_yaml::from_str(&text).expect("the reusable parses as YAML")
}

fn caller() -> Value {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/debian-build.yml");
	let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("the caller is readable: {e}"));
	serde_yaml::from_str(&text).expect("the caller parses as YAML")
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

/// A step that costs runner time: it installs packages, installs a toolchain
/// or builds the package.
fn is_heavy(step: &Value) -> bool {
	let run = run_text(step);
	run.contains("apt-get")
		|| run.contains("rustup ")
		|| run.contains("cargo ")
		|| run.contains("dpkg-buildpackage")
		|| run.contains("dpkg-checkbuilddeps")
		|| run.contains("dpkg-deb")
}

#[test]
fn every_step_that_builds_something_is_behind_the_guard() {
	let spec = reusable();
	for (id, _) in GOVERNED {
		let mut heavy = 0;
		let mut guarded = 0;
		let mut prerequisite = 0;
		for step in steps(&job(&spec, id)) {
			if !is_heavy(&step) {
				continue;
			}
			heavy += 1;
			if if_text(&step).contains(GUARD) {
				guarded += 1;
			} else if step["name"].as_str() == Some(PREREQUISITE) {
				prerequisite += 1;
			}
		}
		assert!(
			heavy > 0,
			"{id} has no heavy step, so this case proves nothing"
		);
		assert_eq!(
			prerequisite, 1,
			"{id} has {prerequisite} heavy steps outside the guard. Exactly one \
			 may be there: the step that installs the git the classifier reads \
			 the judge with. Any other one runs on a pull request the \
			 classifier judged irrelevant, and the classification becomes a \
			 claim."
		);
		assert_eq!(
			heavy,
			guarded + prerequisite,
			"{id} runs {heavy} heavy steps and guards {guarded} of them. An \
			 unguarded one runs on a pull request the classifier judged \
			 irrelevant, and the classification becomes a claim."
		);
	}
}

#[test]
fn the_guard_builds_the_package_when_the_decision_never_arrived() {
	let spec = reusable();
	for (id, _) in GOVERNED {
		for step in steps(&job(&spec, id)) {
			let cond = if_text(&step);
			if !cond.contains("steps.decide.outputs.run") {
				continue;
			}
			assert!(
				cond.contains("!= 'false'"),
				"{id} guards a step with {cond:?}. That skips the package when \
				 the decision is missing, and a missing decision has to build it."
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
			.filter(|r| r.contains("debian-ci-inputs.sh"))
			.collect();
		assert_eq!(
			asked.len(),
			1,
			"{id} asks the classifier {asked:?}; exactly one question per job"
		);
		assert!(
			asked[0].contains("git show \"$BASE_SHA:.github/scripts/checks/debian-ci-inputs.sh\""),
			"{id} does not read the judge from the reviewed base: {}",
			asked[0]
		);
		assert!(
			!asked[0].contains("bash .github/scripts/checks/debian-ci-inputs.sh"),
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

/// The Debian image carries neither git nor the certificates git needs to talk
/// to GitHub over https. The first version of this step installed git alone
/// with `--no-install-recommends`, the checkout then failed with "Problem with
/// the SSL CA cert", and every pull request had a red Debian lane.
#[test]
fn the_image_gets_git_and_the_certificates_before_the_checkout() {
	let spec = reusable();
	for (id, _) in GOVERNED {
		let all = steps(&job(&spec, id));
		let install = all
			.iter()
			.position(|s| run_text(s).contains("apt-get install") && run_text(s).contains("git"))
			.unwrap_or_else(|| panic!("{id} installs git before the checkout"));
		let checkout = all
			.iter()
			.position(|s| {
				s["uses"]
					.as_str()
					.unwrap_or_default()
					.starts_with("actions/checkout")
			})
			.unwrap_or_else(|| panic!("{id} checks the repository out"));
		assert!(install < checkout, "{id} installs git after the checkout");
		let trusted = all
			.iter()
			.position(|s| run_text(s).contains("safe.directory"))
			.unwrap_or_else(|| panic!("{id} trusts the workspace for git"));
		assert!(
			trusted > checkout,
			"{id} trusts the workspace before the checkout, which then replaces the global config"
		);
		assert!(
			run_text(&all[install]).contains("ca-certificates"),
			"{id} installs git without ca-certificates, so the checkout fails with an SSL CA error"
		);
	}
}

#[test]
fn the_range_comes_from_the_event() {
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
fn the_check_names_are_unchanged() {
	let spec = reusable();
	for (id, name) in GOVERNED {
		assert_eq!(
			job(&spec, id)["name"].as_str(),
			Some(name),
			"{id} no longer reports the name it reported before"
		);
	}
	let caller = caller();
	assert_eq!(
		caller["jobs"]["debian"]["uses"].as_str(),
		Some("./.github/workflows/reusable-rust-debian.yml"),
		"the caller no longer calls the reusable under the name the check is \
		 reported with"
	);
}

/// The lane still watches the whole test tree, and the classifier is watched
/// too, so the rule that names the classifier is reachable.
#[test]
fn the_caller_watches_the_classifier_and_the_whole_test_tree() {
	let caller = caller();
	for event in ["push", "pull_request"] {
		let paths: Vec<String> = caller["on"][event]["paths"]
			.as_sequence()
			.map(|items| {
				items
					.iter()
					.filter_map(|v| v.as_str().map(str::to_string))
					.collect()
			})
			.unwrap_or_default();
		assert!(
			paths.iter().any(|p| p == "tests/**"),
			"{event} no longer watches the test tree. The classifier decides \
			 whether to build the package, it never decides whether to run \
			 the lane: #1295 rode twenty pull requests because the lane that \
			 compiles with debian/rules' feature set did not run at all."
		);
		assert!(
			paths
				.iter()
				.any(|p| p == ".github/scripts/checks/debian-ci-inputs.sh"),
			"{event} does not watch the classifier, so the rule that names it \
			 is dead: a pull request that edits the judge would never reach \
			 the lane that reads it."
		);
	}
}
