//! Every workflow that runs on `pull_request` supersedes its own stale runs,
//! and only for `pull_request`.
//!
//! Measured on 2026-10-25: 3 of the 21 non-reusable workflows had a
//! `concurrency:` block and 13 more ran on pull requests or pushes without
//! one, so a new push to a pull request left the run for the old commit
//! consuming runners. The standard (`ai-context/standards/ci`, "A check costs
//! what it learns", rule 1) allows `cancel-in-progress` for `pull_request`
//! only and never for a `merge_group`.
//!
//! Two properties are enforced per workflow, because either one alone is a
//! trap:
//!
//! 1. `cancel-in-progress` is an expression that is true only for
//!    `pull_request`. A literal `true` would cancel a push to `develop` or a
//!    queued merge group mid-run.
//! 2. `group` cannot be shared by events that are not pull requests. With a
//!    shared group GitHub keeps at most one pending run and drops the older
//!    ones even with `cancel-in-progress: false`, and `branch health
//!    (develop)` waits for the run of one specific develop commit, so a
//!    dropped run would leave it red for good. The group must carry
//!    `github.run_id` or `github.event_name`.
//!
//! The checker is exercised against planted YAML first, so a rule that could
//! not fail would show up here instead of passing in silence.

use std::fs;
use std::path::Path;

use serde_yaml::Value;

const PR_ONLY_CANCEL: &str = "github.event_name == 'pull_request'";

/// Whether the workflow's `on:` includes `pull_request`. `on` may be a
/// string, a sequence or a mapping, and a YAML 1.1 reader may turn the key
/// into a boolean, so both spellings are looked up.
fn runs_on_pull_request(doc: &Value) -> bool {
	let on = doc
		.get("on")
		.or_else(|| doc.get(Value::Bool(true)))
		.unwrap_or(&Value::Null);
	match on {
		Value::String(s) => s == "pull_request",
		Value::Sequence(items) => items.iter().any(|v| v.as_str() == Some("pull_request")),
		Value::Mapping(map) => map.contains_key("pull_request"),
		_ => false,
	}
}

/// `Err` carries the reason a workflow breaks the rule; `Ok` also covers the
/// workflows that do not run on `pull_request` and so owe nothing.
fn check(doc: &Value) -> Result<(), String> {
	if !runs_on_pull_request(doc) {
		return Ok(());
	}
	let Some(block) = doc.get("concurrency") else {
		return Err("runs on pull_request but has no workflow-level `concurrency:`".into());
	};
	let cancel = match block.get("cancel-in-progress") {
		Some(Value::String(s)) => s.clone(),
		Some(other) => {
			return Err(format!(
				"`cancel-in-progress` is the literal {other:?}; it must be an expression \
				 that cancels only for pull_request, e.g. `${{{{ {PR_ONLY_CANCEL} }}}}`"
			))
		}
		None => return Err("`concurrency:` has no `cancel-in-progress`".into()),
	};
	if !cancel.contains(PR_ONLY_CANCEL) {
		return Err(format!(
			"`cancel-in-progress` is `{cancel}`; it must contain `{PR_ONLY_CANCEL}` so push, \
			 merge_group and schedule runs are never cancelled"
		));
	}
	let group = match block.get("group") {
		Some(Value::String(s)) => s.as_str(),
		_ => return Err("`concurrency.group` is missing or not a string".into()),
	};
	if !group.contains("github.run_id") && !group.contains("github.event_name") {
		return Err(format!(
			"`concurrency.group` is `{group}`; events that are not pull requests would share it, \
			 so GitHub would drop pending runs. Include `github.run_id` or `github.event_name`"
		));
	}
	Ok(())
}

fn parse(text: &str) -> Value {
	serde_yaml::from_str(text).expect("workflow YAML parses")
}

#[test]
fn every_pull_request_workflow_cancels_only_pull_request_runs() {
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
	let mut failures = Vec::new();
	let mut checked = 0;
	for entry in fs::read_dir(&dir).expect("read .github/workflows") {
		let path = entry.expect("dir entry").path();
		let name = path.file_name().unwrap().to_string_lossy().into_owned();
		if !name.ends_with(".yml") || name.starts_with("reusable-") {
			continue;
		}
		let doc = parse(&fs::read_to_string(&path).expect("read workflow"));
		if runs_on_pull_request(&doc) {
			checked += 1;
		}
		if let Err(why) = check(&doc) {
			failures.push(format!("{name}: {why}"));
		}
	}
	assert!(
		checked >= 10,
		"only {checked} workflows were recognised as running on pull_request; the reader has stopped seeing `on:`"
	);
	assert!(failures.is_empty(), "\n{}\n", failures.join("\n"));
}

#[test]
fn the_new_pattern_and_the_podman_lane_pattern_both_pass() {
	let new = "on:\n  pull_request:\n  push:\nconcurrency:\n  group: ${{ github.workflow }}-${{ github.event_name == 'pull_request' && github.ref || github.run_id }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\njobs: {}\n";
	let lane = "on:\n  pull_request:\nconcurrency:\n  group: podman-lane-${{ github.event_name }}-${{ github.ref }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\njobs: {}\n";
	assert_eq!(check(&parse(new)), Ok(()));
	assert_eq!(check(&parse(lane)), Ok(()));
}

#[test]
fn a_workflow_without_concurrency_fails() {
	let planted = "on: [pull_request, push]\njobs: {}\n";
	let err = check(&parse(planted)).unwrap_err();
	assert!(err.contains("no workflow-level `concurrency:`"), "{err}");
}

#[test]
fn unconditional_cancel_in_progress_fails() {
	let bool_true = "on:\n  pull_request:\nconcurrency:\n  group: ${{ github.workflow }}-${{ github.run_id }}\n  cancel-in-progress: true\njobs: {}\n";
	let err = check(&parse(bool_true)).unwrap_err();
	assert!(err.contains("literal"), "{err}");
	let other_expr = "on:\n  pull_request:\nconcurrency:\n  group: ${{ github.workflow }}-${{ github.run_id }}\n  cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}\njobs: {}\n";
	let err = check(&parse(other_expr)).unwrap_err();
	assert!(err.contains("must contain"), "{err}");
}

#[test]
fn a_group_shared_with_non_pull_request_events_fails() {
	let planted = "on:\n  pull_request:\n  push:\nconcurrency:\n  group: ${{ github.workflow }}-${{ github.ref }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\njobs: {}\n";
	let err = check(&parse(planted)).unwrap_err();
	assert!(err.contains("would share it"), "{err}");
}

#[test]
fn a_workflow_that_never_runs_on_pull_request_owes_nothing() {
	let planted = "on:\n  schedule:\n    - cron: '0 6 * * *'\njobs: {}\n";
	assert_eq!(check(&parse(planted)), Ok(()));
}

const RUN_DISCRIMINANT: &str =
	"${{ github.event_name == 'pull_request' && github.ref || github.run_id }}";
const CANCEL_EXPRESSION: &str = "${{ github.event_name == 'pull_request' }}";

fn check_reusable(doc: &Value, prefix: &str, suffix: &str) -> Result<(), String> {
	let expected = format!("{prefix}-${{{{ github.workflow }}}}-{RUN_DISCRIMINANT}{suffix}");
	if doc["concurrency"]["group"].as_str() != Some(&expected) {
		return Err(format!("reusable group must be {expected}"));
	}
	if doc["concurrency"]["cancel-in-progress"].as_str() != Some(CANCEL_EXPRESSION) {
		return Err("reusable must cancel only PR runs".into());
	}
	Ok(())
}

fn workflow(name: &str) -> Value {
	let path = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join(".github/workflows")
		.join(format!("{name}.yml"));
	parse(&fs::read_to_string(path).expect("read workflow"))
}

#[test]
fn reusables_preserve_non_pr_runs_and_separate_sibling_calls() {
	for (name, suffix) in [
		(
			"branch-health",
			"-${{ inputs.workflow }}-${{ inputs.branch }}",
		),
		("dco", ""),
		("dependabot-freshness", ""),
		("installer-contract", "-${{ inputs.install-script }}"),
		("line-limit", ""),
		("main-guard", ""),
		("powershell-ci", "-${{ inputs.paths }}"),
		("rust-audit", "-${{ inputs.working-directory }}"),
		("rust-ci", "-${{ inputs.working-directory }}"),
		(
			"rust-debian",
			"-${{ inputs.arch }}-${{ inputs.package-name }}",
		),
		(
			"rust-fuzz",
			"-${{ inputs.working-directory }}-${{ inputs.build-only }}",
		),
		("rust-supply-chain", "-${{ inputs.working-directory }}"),
		("schedule-freshness", "-${{ inputs.workflow }}"),
		("shell-ci", "-${{ inputs.working-directory }}"),
		("workflow-lint", ""),
	] {
		let doc = workflow(&format!("reusable-{name}"));
		assert_eq!(check_reusable(&doc, name, suffix), Ok(()), "{name}");
	}
}

#[test]
fn reusable_group_cannot_collide_with_its_caller() {
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
	for entry in fs::read_dir(dir).unwrap() {
		let path = entry.unwrap().path();
		if path.extension().and_then(|s| s.to_str()) != Some("yml") {
			continue;
		}
		let doc = parse(&fs::read_to_string(&path).unwrap());
		let Some(jobs) = doc["jobs"].as_mapping() else {
			continue;
		};
		for job in jobs.values() {
			let Some(target) = job["uses"]
				.as_str()
				.and_then(|s| s.strip_prefix("./.github/workflows/"))
			else {
				continue;
			};
			let inner = workflow(target.trim_end_matches(".yml"));
			assert_ne!(
				doc["concurrency"]["group"],
				inner["concurrency"]["group"],
				"{} -> {target}",
				path.display()
			);
		}
	}
}

#[test]
fn reusable_checker_rejects_each_unsafe_component() {
	let doc = workflow("reusable-branch-health");
	for (field, value, diagnostic) in [
		("group", "branch-health-${{ github.workflow }}-${{ github.ref }}", "reusable group"),
		("group", "${{ github.workflow }}-${{ github.event_name == 'pull_request' && github.ref || github.run_id }}", "reusable group"),
		("group", "branch-health-${{ github.workflow }}-${{ github.event_name == 'pull_request' && github.ref || github.run_id }}-${{ inputs.workflow }}", "reusable group"),
		("cancel-in-progress", "true", "cancel only PR"),
	] {
		let mut planted = doc.clone();
		planted["concurrency"][field] = Value::String(value.into());
		let err = check_reusable(&planted, "branch-health", "-${{ inputs.workflow }}-${{ inputs.branch }}").unwrap_err();
		assert!(err.contains(diagnostic), "{err}");
	}
}

#[test]
fn supply_chain_filters_match_dependency_and_generator_inputs() {
	let doc = workflow("supply-chain");
	let expected: Value = serde_yaml::to_value([
		".cargo/**",
		"Cargo.toml",
		"Cargo.lock",
		"about.toml",
		"about.hbs",
		".github/workflows/supply-chain.yml",
		".github/workflows/reusable-rust-supply-chain.yml",
	])
	.unwrap();
	for event in ["push", "pull_request"] {
		assert_eq!(
			doc["on"][event]["paths"], expected,
			"{event} supply-chain paths"
		);
	}
}

#[test]
fn publishers_and_watchers_serialize_without_cancelling() {
	for (name, group) in [
		("release", "release-${{ github.repository }}-${{ github.event_name == 'workflow_dispatch' && inputs.tag || github.ref_name }}"),
		("podman-lane-develop-nightly", "${{ github.workflow }}-develop"),
		("pin-watch", "${{ github.workflow }}"),
		("podman-watch", "${{ github.workflow }}"),
	] {
		let doc = workflow(name);
		assert_eq!(doc["concurrency"]["group"].as_str(), Some(group), "{name} serialization group");
		assert_eq!(doc["concurrency"]["cancel-in-progress"], Value::Bool(false), "{name} must not cancel an active run");
	}
}

#[path = "workflow_concurrency/release.rs"]
mod release;
