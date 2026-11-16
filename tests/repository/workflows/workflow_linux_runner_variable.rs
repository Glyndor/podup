//! Every unprivileged Linux job takes its runner from the one repository variable
//! `CI_RUNNER`, and no privileged job does.
//!
//! A job pinned to `ubuntu-latest` or `ubuntu-24.04` ignores the variable, so a
//! self-hosted runner set through it would quietly leave that job on a GitHub
//! one. The other way round is worse: a job that holds a secret or a write
//! permission must not share a runner with the code of a pull request from a
//! fork. This fails when an unprivileged Linux `runs-on` is written as the fixed
//! literal again, and when a job that is privileged by what it contains reads the
//! variable.

use std::fs;
use std::path::Path;

use serde_yaml::Value;

/// `(file, job, literal, why it stays on a GitHub-hosted runner)`. An empty job
/// means every job of the file.
const FIXED: [(&str, &str, &str, &str); 7] = [
	(
		"release.yml",
		"",
		"ubuntu-latest",
		"it builds, signs and publishes the release",
	),
	(
		"podman-lane.yml",
		"podman-vm",
		"ubuntu-24.04",
		"it boots QEMU with /dev/kvm and sudo, and is a required check",
	),
	(
		"podman-lane-develop-nightly.yml",
		"podman-vm",
		"ubuntu-24.04",
		"the same VM legs, run nightly against develop",
	),
	(
		"audit.yml",
		"audit-develop",
		"ubuntu-latest",
		"it dispatches a workflow with actions: write",
	),
	(
		"audit.yml",
		"report",
		"ubuntu-latest",
		"it opens issues with issues: write",
	),
	(
		"pin-watch.yml",
		"watch",
		"ubuntu-24.04",
		"it opens issues with issues: write",
	),
	(
		"podman-watch.yml",
		"watch",
		"ubuntu-24.04",
		"it opens issues with issues: write",
	),
];

/// Permissions that make a job privileged when set to `write`.
const WRITE_KEYS: [&str; 6] = [
	"contents",
	"packages",
	"id-token",
	"attestations",
	"pull-requests",
	"issues",
];

const LITERALS: [&str; 2] = ["ubuntu-latest", "ubuntu-24.04"];

fn workflows() -> Vec<(String, Value)> {
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
	let mut out = Vec::new();
	for entry in fs::read_dir(&dir).expect("the workflows directory is readable") {
		let path = entry.expect("a directory entry").path();
		if path.extension().is_some_and(|e| e == "yml") {
			let name = path.file_name().unwrap().to_string_lossy().into_owned();
			let text = fs::read_to_string(&path).expect("a workflow is readable");
			let doc: Value = serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
			out.push((name, doc));
		}
	}
	out.sort_by(|a, b| a.0.cmp(&b.0));
	out
}

fn runs_on(job: &Value) -> String {
	match job.get("runs-on") {
		Some(Value::String(s)) => s.clone(),
		Some(other) => serde_yaml::to_string(other).expect("a runs-on value serialises"),
		None => String::new(),
	}
}

fn fixed_by_exception(file: &str, job: &str, literal: &str) -> bool {
	FIXED
		.iter()
		.any(|(f, j, l, _)| *f == file && (*j == job || j.is_empty()) && *l == literal)
}

/// Why the job is privileged, or `None`. `workflow` supplies the permissions a job
/// that sets none inherits.
fn privilege(workflow: &Value, job: &Value) -> Option<String> {
	let perms = job
		.get("permissions")
		.or_else(|| workflow.get("permissions"));
	if let Some(Value::Mapping(map)) = perms {
		for key in WRITE_KEYS {
			if map.get(key).and_then(Value::as_str) == Some("write") {
				return Some(format!("permissions: {key}: write"));
			}
		}
	}
	if matches!(perms, Some(Value::String(s)) if s == "write-all") {
		return Some("permissions: write-all".to_string());
	}
	if job.get("secrets").is_some() {
		return Some("it hands secrets to a reusable workflow".to_string());
	}
	let text = serde_yaml::to_string(job).expect("a job serialises");
	let mut rest = text.as_str();
	while let Some(at) = rest.find("secrets.") {
		let name: String = rest[at + 8..]
			.chars()
			.take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
			.collect();
		if name != "GITHUB_TOKEN" {
			return Some(format!("it reads secrets.{name}"));
		}
		rest = &rest[at + 8..];
	}
	None
}

/// One line per problem found in `files`.
fn problems(files: &[(String, Value)]) -> Vec<String> {
	let mut out = Vec::new();
	for (file, doc) in files {
		let Some(Value::Mapping(jobs)) = doc.get("jobs") else {
			continue;
		};
		for (name, job) in jobs {
			let name = name.as_str().unwrap_or("?");
			let value = runs_on(job);
			if LITERALS.contains(&value.as_str()) && !fixed_by_exception(file, name, &value) {
				out.push(format!(
					"{file}::{name}: runs-on: {value} ignores CI_RUNNER; write \
					 runs-on: ${{{{ fromJSON(vars.CI_RUNNER || '\"ubuntu-latest\"') }}}}"
				));
			}
			if value.contains("CI_RUNNER") {
				if let Some(why) = privilege(doc, job) {
					out.push(format!(
						"{file}::{name}: reads CI_RUNNER but is privileged ({why}); \
						 pin it to a GitHub runner and list it in FIXED"
					));
				}
			}
		}
	}
	out
}

#[test]
fn no_unprivileged_linux_job_is_pinned_and_no_privileged_one_reads_the_variable() {
	let found = problems(&workflows());
	assert!(found.is_empty(), "\n{}\n", found.join("\n"));
}

#[test]
fn the_fixed_list_still_names_a_job_that_needs_it() {
	// A stale entry would let a fixed runner back in unnoticed.
	let files = workflows();
	for (file, job, literal, why) in FIXED {
		let doc = &files
			.iter()
			.find(|(name, _)| name == file)
			.unwrap_or_else(|| panic!("{file} is listed ({why}) but does not exist"))
			.1;
		let Some(Value::Mapping(jobs)) = doc.get("jobs") else {
			panic!("{file} has no jobs");
		};
		let pinned = jobs.iter().any(|(name, spec)| {
			(job.is_empty() || name.as_str() == Some(job)) && runs_on(spec) == literal
		});
		assert!(
			pinned,
			"{file}::{job} no longer pins {literal}; drop the entry ({why})"
		);
	}
}

fn parse(text: &str) -> Vec<(String, Value)> {
	vec![(
		"example.yml".to_string(),
		serde_yaml::from_str(text).unwrap(),
	)]
}

#[test]
fn a_pinned_unprivileged_job_is_reported() {
	let files = parse("jobs:\n  a:\n    runs-on: ubuntu-latest\n");
	assert_eq!(problems(&files).len(), 1);
}

#[test]
fn a_privileged_job_that_reads_the_variable_is_reported() {
	let var = "${{ fromJSON(vars.CI_RUNNER || '\"ubuntu-latest\"') }}";
	for body in [
		"permissions:\n      issues: write\n    steps: []",
		"permissions:\n      contents: write\n    steps: []",
		"steps:\n      - run: echo ${{ secrets.TOKEN }}",
		"uses: ./.github/workflows/x.yml\n    secrets: inherit",
	] {
		let files = parse(&format!("jobs:\n  a:\n    runs-on: {var}\n    {body}\n"));
		let found = problems(&files);
		assert_eq!(found.len(), 1, "{body}: {found:?}");
		assert!(found[0].contains("privileged"), "{found:?}");
	}
}

#[test]
fn an_unprivileged_job_on_the_variable_is_fine() {
	let var = "${{ fromJSON(vars.CI_RUNNER || '\"ubuntu-latest\"') }}";
	let files = parse(&format!(
		"permissions:\n  contents: read\njobs:\n  a:\n    runs-on: {var}\n    steps:\n      - run: echo ${{{{ secrets.GITHUB_TOKEN }}}}\n"
	));
	assert!(problems(&files).is_empty());
}
