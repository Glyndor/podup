//! The top-level `jobs:` section, seen from the command line.
//!
//! A job is a container that runs to completion. `config` shows it, `audit`
//! checks it, `run` starts it and `up` never does. A schedule is validated and
//! reported once per job, because podup does not fire it. Every refusal names
//! the job and happens before an engine is contacted.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const WITH_JOBS: &str = "\
services:
  web:
    image: nginx
jobs:
  nightly:
    image: alpine
    command: [echo, hi]
    triggers:
      manual: true
      schedule:
        - '0 1 * * *'
        - cron: '0 2 * * *'
          timezone: Europe/Berlin
          concurrency: queue
        - '0 3 * * *'
  weekly:
    image: alpine
    triggers:
      schedule: ['0 4 * * 0']
  manual-only:
    image: alpine
    triggers: {manual: true}
";

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Write `contents` as `compose.yaml` in a temp dir of its own.
fn compose(contents: &str) -> (TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("create temp dir");
	let path = dir.path().join("compose.yaml");
	fs::write(&path, contents).expect("write compose file");
	(dir, path)
}

fn podup(file: &Path, args: &[&str]) -> Output {
	Command::new(bin())
		.args(["-f", file.to_str().unwrap()])
		.args(args)
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup")
}

fn stderr(out: &Output) -> String {
	String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
	String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn config_prints_a_job_with_its_triggers() {
	let (_dir, file) = compose(WITH_JOBS);
	let out = podup(&file, &["config"]);
	assert!(out.status.success(), "{}", stderr(&out));
	let text = stdout(&out);
	assert!(
		text.contains("services:") && text.contains("web:"),
		"{text}"
	);
	assert!(
		text.contains("jobs:") && text.contains("nightly:"),
		"{text}"
	);
	// (the YAML renderer quotes `true`, as it does for every boolean it prints;
	// the JSON test below pins the value itself)
	assert!(text.contains("manual:"), "{text}");
	assert!(text.contains("timezone: Europe/Berlin"), "{text}");
	assert!(text.contains("concurrency: queue"), "{text}");
}

#[test]
fn config_json_carries_the_triggers_as_data() {
	let (_dir, file) = compose(WITH_JOBS);
	let out = podup(&file, &["config", "--format", "json"]);
	assert!(out.status.success(), "{}", stderr(&out));
	let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("config json");
	assert_eq!(value["jobs"]["nightly"]["triggers"]["manual"], true);
	let schedule = &value["jobs"]["nightly"]["triggers"]["schedule"];
	assert_eq!(schedule[0], "0 1 * * *");
	assert_eq!(schedule[1]["cron"], "0 2 * * *");
	assert_eq!(value["jobs"]["weekly"]["image"], "alpine");
}

#[test]
fn config_warns_once_per_job_with_a_schedule() {
	let (_dir, file) = compose(WITH_JOBS);
	let err = stderr(&podup(&file, &["config"]));
	assert_eq!(
		err.matches("job 'nightly'").count(),
		1,
		"three schedule entries, one notice: {err}"
	);
	assert_eq!(err.matches("job 'weekly'").count(), 1, "{err}");
	assert!(
		!err.contains("job 'manual-only'"),
		"no schedule, no notice: {err}"
	);
	assert!(err.contains("does not fire"), "{err}");
}

#[test]
fn config_quiet_still_warns() {
	let (_dir, file) = compose(WITH_JOBS);
	let out = podup(&file, &["config", "--quiet"]);
	assert!(out.status.success());
	assert!(stderr(&out).contains("job 'nightly'"), "{}", stderr(&out));
	assert!(stdout(&out).is_empty());
}

#[test]
fn a_job_behind_an_inactive_profile_is_hidden_and_silent() {
	let yaml = "services:\n  web:\n    image: nginx\njobs:\n  ops:\n    image: alpine\n    profiles: [ops]\n    triggers:\n      schedule: ['0 1 * * *']\n";
	let (_dir, file) = compose(yaml);
	let hidden = podup(&file, &["config"]);
	assert!(!stdout(&hidden).contains("ops:"), "{}", stdout(&hidden));
	assert!(
		!stderr(&hidden).contains("job 'ops'"),
		"{}",
		stderr(&hidden)
	);
	let shown = podup(&file, &["--profile", "ops", "config"]);
	assert!(stdout(&shown).contains("ops:"), "{}", stdout(&shown));
	assert!(stderr(&shown).contains("job 'ops'"), "{}", stderr(&shown));
}

#[test]
fn variables_are_interpolated_inside_a_job() {
	let yaml = "jobs:\n  j:\n    image: ${JOB_IMAGE}\n    triggers: {manual: true}\n";
	let (_dir, file) = compose(yaml);
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "config"])
		.env("JOB_IMAGE", "registry.example/job:7")
		.output()
		.unwrap();
	assert!(out.status.success(), "{}", stderr(&out));
	assert!(
		stdout(&out).contains("registry.example/job:7"),
		"{}",
		stdout(&out)
	);
}

#[test]
fn a_required_variable_in_a_job_is_enforced() {
	let yaml = "jobs:\n  j:\n    image: ${JOB_IMAGE_NOT_SET:?the job image is required}\n    triggers: {manual: true}\n";
	let (_dir, file) = compose(yaml);
	let out = podup(&file, &["config"]);
	assert!(!out.status.success());
	assert!(
		stderr(&out).contains("the job image is required"),
		"{}",
		stderr(&out)
	);
}

#[test]
fn jobs_of_several_files_are_merged_and_an_include_brings_its_jobs() {
	let dir = tempfile::tempdir().unwrap();
	let base = dir.path().join("base.yaml");
	let over = dir.path().join("over.yaml");
	let inc = dir.path().join("inc.yaml");
	fs::write(
		&base,
		"include: [inc.yaml]\nservices:\n  web:\n    image: nginx\njobs:\n  backup:\n    image: alpine:3\n    triggers: {manual: true}\n",
	)
	.unwrap();
	fs::write(
		&over,
		"jobs:\n  backup:\n    image: alpine:4\n    triggers: {manual: true}\n",
	)
	.unwrap();
	fs::write(
		&inc,
		"jobs:\n  included:\n    image: busybox\n    triggers: {manual: true}\n",
	)
	.unwrap();
	let out = Command::new(bin())
		.args([
			"-f",
			base.to_str().unwrap(),
			"-f",
			over.to_str().unwrap(),
			"config",
		])
		.output()
		.unwrap();
	assert!(out.status.success(), "{}", stderr(&out));
	let text = stdout(&out);
	assert!(
		text.contains("alpine:4") && !text.contains("alpine:3"),
		"{text}"
	);
	assert!(
		text.contains("included:") && text.contains("busybox"),
		"{text}"
	);
}

#[test]
fn an_empty_or_null_jobs_section_is_the_same_as_none() {
	for jobs in ["jobs: {}", "jobs:"] {
		let (_dir, file) = compose(&format!("services:\n  web:\n    image: nginx\n{jobs}\n"));
		let out = podup(&file, &["config"]);
		assert!(out.status.success(), "{jobs}: {}", stderr(&out));
		assert!(!stdout(&out).contains("jobs:"), "{jobs}: {}", stdout(&out));
		assert!(stderr(&out).is_empty(), "{jobs}: {}", stderr(&out));
	}
}

#[test]
fn a_file_with_only_jobs_is_a_valid_project() {
	let (_dir, file) = compose("jobs:\n  j:\n    image: alpine\n    triggers: {manual: true}\n");
	let out = podup(&file, &["config"]);
	assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn images_and_profiles_listings_include_jobs() {
	let yaml = "services:\n  web:\n    image: nginx\njobs:\n  j:\n    image: job-image\n    profiles: [ops]\n    triggers: {manual: true}\n";
	let (_dir, file) = compose(yaml);
	let images = stdout(&podup(&file, &["--profile", "ops", "config", "--images"]));
	assert!(
		images.contains("job-image") && images.contains("nginx"),
		"{images}"
	);
	let profiles = stdout(&podup(&file, &["--profile", "ops", "config", "--profiles"]));
	assert!(profiles.contains("ops"), "{profiles}");
}

/// Each refusal: the file, and what the message must say. All happen at
/// `config`, and every one names the job.
const REFUSALS: &[(&str, &str)] = &[
	(
		"jobs:\n  j:\n    image: alpine\n",
		"job 'j' declares no triggers",
	),
	(
		"jobs:\n  j:\n    image: alpine\n    triggers: {}\n",
		"job 'j' declares no trigger",
	),
	(
		"jobs:\n  j:\n    image: alpine\n    triggers: []\n",
		"`triggers` of job 'j' must be a mapping",
	),
	(
		"jobs:\n  j:\n    image: alpine\n    triggers:\n      schedule: '0 1 * * *'\n",
		"`triggers.schedule` of job 'j' must be a list",
	),
	(
		"jobs:\n  j:\n    image: alpine\n    triggers:\n      schedule: [{cron: '* * * * *', concurrency: allow}]\n",
		"`concurrency` in schedule entry 1 of job 'j'",
	),
	(
		"jobs:\n  j:\n    image: alpine\n    triggers:\n      schedule: [\"0 1 * * *\\nExecStart=/bin/sh\"]\n",
		"contains a control character",
	),
	(
		"services:\n  x:\n    image: a\njobs:\n  x:\n    image: a\n    triggers: {manual: true}\n",
		"the name 'x' is declared as a service and as a job",
	),
	(
		"services:\n  s:\n    image: a\n    depends_on: [j]\njobs:\n  j:\n    image: a\n    triggers: {manual: true}\n",
		"service 's' depends on job 'j'",
	),
	(
		"services:\n  s:\n    image: a\njobs:\n  j:\n    image: a\n    extends: s\n    triggers: {manual: true}\n",
		"job 'j' extends 's', which is a service",
	),
	(
		"services:\n  s:\n    image: a\n    extends: j\njobs:\n  j:\n    image: a\n    triggers: {manual: true}\n",
		"service 's' extends 'j', which is a job",
	),
	(
		"jobs:\n  a:\n    image: x\n    extends: b\n    triggers: {manual: true}\n  b:\n    image: x\n    extends: a\n    triggers: {manual: true}\n",
		"circular extends",
	),
	(
		"jobs:\n  a:\n    image: x\n    depends_on: [b]\n    triggers: {manual: true}\n  b:\n    image: x\n    depends_on: [a]\n    triggers: {manual: true}\n",
		"circular dependency",
	),
];

#[test]
fn config_refuses_a_bad_jobs_section_and_names_the_job() {
	for (yaml, expected) in REFUSALS {
		let (_dir, file) = compose(yaml);
		let out = podup(&file, &["config"]);
		assert!(!out.status.success(), "must be refused:\n{yaml}");
		let err = stderr(&out);
		assert!(
			err.contains(expected),
			"expected {expected:?} for:\n{yaml}\ngot: {err}"
		);
		assert!(
			!err.contains("ExecStart"),
			"the hostile text is never echoed: {err}"
		);
	}
}

#[test]
fn a_hostile_job_name_never_reaches_the_terminal_raw() {
	let yaml = "jobs:\n  \"j\\e[31m\\nx\":\n    image: alpine\n";
	let (_dir, file) = compose(yaml);
	let out = podup(&file, &["config"]);
	assert!(!out.status.success());
	let err = stderr(&out);
	assert!(!err.contains('\u{1b}'), "{err:?}");
	assert!(err.contains("j\\u{1b}[31m\\nx"), "{err}");
}
