//! The refusal of a compose file that declares a top-level `jobs:` section.
//!
//! Split out from `jobs.rs`; the module is imported here rather than through
//! the parent, so the scenarios read as the behaviour of the parse path.

use crate::compose::parse_files_with_env_files;
use crate::compose::parse_str;
use crate::compose::types::ComposeFile;
use crate::error::ComposeError;

/// A compose file with one job, the smallest shape the section takes.
const ONE_JOB: &str = r#"
services:
  web:
    image: nginx
jobs:
  backup:
    image: alpine
    triggers:
      - type: schedule
        schedule: daily
"#;

/// The error a file with jobs must produce.
fn refusal(yaml: &str) -> String {
	let err = parse_str(yaml).expect_err("a file with jobs must be refused");
	assert!(
		matches!(err, ComposeError::Unsupported(_)),
		"the refusal must be an unsupported-feature error, got {err:?}"
	);
	err.to_string()
}

#[test]
fn a_file_with_one_job_is_refused() {
	let message = refusal(ONE_JOB);
	assert!(
		message.contains(
			"jobs: the top-level jobs section is not supported by podup yet; remove it or run \
			 those containers with another tool"
		),
		"the refusal must state what is wrong and what to do; got: {message}"
	);
}

#[test]
fn the_refusal_names_the_first_job() {
	let message = refusal(ONE_JOB);
	assert!(
		message.contains("backup"),
		"the refusal must name the job to remove; got: {message}"
	);
}

#[test]
fn the_refusal_names_the_first_job_when_several_are_declared() {
	let yaml = r#"
services:
  web:
    image: nginx
jobs:
  zeta:
    image: alpine
  alpha:
    image: alpine
"#;
	let message = refusal(yaml);
	assert!(
		message.contains("zeta"),
		"the first declared job must be the one named; got: {message}"
	);
	assert!(
		!message.contains("alpha"),
		"only the first job is named; got: {message}"
	);
}

#[test]
fn an_empty_jobs_map_is_accepted() {
	let yaml = "services:\n  web:\n    image: nginx\njobs: {}\n";
	let file = parse_str(yaml).expect("an empty jobs map declares nothing");
	assert_eq!(file.jobs.as_ref().map(|jobs| jobs.len()), Some(0));
}

#[test]
fn a_null_jobs_value_is_accepted() {
	let yaml = "services:\n  web:\n    image: nginx\njobs:\n";
	let file = parse_str(yaml).expect("a null jobs value declares nothing");
	assert!(file.jobs.is_none());
}

#[test]
fn a_file_without_jobs_is_unchanged() {
	let yaml = "services:\n  web:\n    image: nginx\n";
	let file = parse_str(yaml).expect("a file without jobs parses");
	assert!(file.jobs.is_none());
	assert_eq!(file.services.len(), 1);
	assert_eq!(file.services["web"].image.as_deref(), Some("nginx"));
}

#[test]
fn the_jobs_key_is_not_reported_as_an_unknown_top_level_key() {
	// `jobs:` is a key the Compose Specification defines, so it must not fall
	// into the unknown-key bucket that only warns: podup refuses it outright.
	let yaml = "services:\n  web:\n    image: nginx\njobs: {}\n";
	let file = parse_str(yaml).expect("an empty jobs map declares nothing");
	assert!(
		!file.extensions.contains_key("jobs"),
		"the jobs key must not land in the unknown-key bucket"
	);
}

#[test]
fn a_default_compose_file_declares_no_job() {
	assert!(ComposeFile::default().jobs.is_none());
}

#[test]
fn a_file_on_disk_with_a_job_is_refused() {
	// The path every command takes: the file is read from disk, not parsed
	// from a string.
	let dir = tempfile::tempdir().expect("create temp dir");
	let path = dir.path().join("compose.yaml");
	std::fs::write(&path, ONE_JOB).expect("write compose file");

	let err = parse_files_with_env_files(&[path], &[]).expect_err("a file with jobs is refused");
	assert!(
		err.to_string().contains("backup"),
		"the refusal must name the job to remove; got: {err}"
	);
}

#[test]
fn a_job_declared_by_an_included_file_is_refused() {
	// The job travels with the include, so the parent file cannot look
	// job-free and let the job through.
	let dir = tempfile::tempdir().expect("create temp dir");
	let parent = dir.path().join("compose.yaml");
	let child = dir.path().join("child.yaml");
	std::fs::write(
		&parent,
		"include:\n  - child.yaml\nservices:\n  web:\n    image: nginx\n",
	)
	.expect("write parent compose file");
	std::fs::write(&child, "jobs:\n  backup:\n    image: alpine\n").expect("write included file");

	let err = parse_files_with_env_files(&[parent], &[]).expect_err("a file with jobs is refused");
	assert!(
		err.to_string().contains("backup"),
		"the refusal must name the job the included file declares; got: {err}"
	);
}
