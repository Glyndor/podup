//! The views a command works on to treat a job as a service.

use super::{run_view, with_jobs_as_services};
use crate::compose::parse_str;
use crate::error::ComposeError;

const FILE: &str = "\
services:
  db:
    image: postgres
  web:
    image: nginx
jobs:
  migrate:
    image: alpine
    depends_on: [db]
    triggers: {manual: true}
  report:
    image: alpine
    depends_on:
      migrate:
        condition: service_completed_successfully
    triggers: {manual: true}
  unrelated:
    image: alpine
    triggers: {manual: true}
  scheduled-only:
    image: alpine
    triggers:
      manual: false
      schedule: ['0 3 * * *']
  needs-closed:
    image: alpine
    depends_on: [scheduled-only]
    triggers: {manual: true}
";

fn names(file: &crate::compose::types::ComposeFile) -> Vec<&str> {
	file.services.keys().map(String::as_str).collect()
}

#[test]
fn the_run_view_holds_the_services_the_job_and_what_it_reaches() {
	let file = parse_str(FILE).unwrap();
	let view = run_view(&file, "report").unwrap();
	assert_eq!(view.jobs, ["report", "migrate"]);
	let mut got = names(&view.file);
	got.sort_unstable();
	assert_eq!(got, ["db", "migrate", "report", "web"]);
	assert!(view.file.jobs.is_empty(), "the view has no jobs left");
}

#[test]
fn a_job_nobody_asked_for_is_not_in_the_view() {
	let file = parse_str(FILE).unwrap();
	let view = run_view(&file, "migrate").unwrap();
	assert!(!names(&view.file).contains(&"unrelated"));
	assert!(!names(&view.file).contains(&"report"));
}

#[test]
fn run_view_leaves_the_original_file_alone() {
	let file = parse_str(FILE).unwrap();
	let _ = run_view(&file, "report").unwrap();
	assert_eq!(file.jobs.len(), 5);
	assert_eq!(file.services.len(), 2);
}

#[test]
fn a_job_with_manual_false_cannot_be_run() {
	let file = parse_str(FILE).unwrap();
	let err = run_view(&file, "scheduled-only")
		.err()
		.expect("manual is false");
	assert!(matches!(err, ComposeError::Job(_)), "{err:?}");
	let message = err.to_string();
	assert!(
		message.contains("job 'scheduled-only' sets `triggers.manual` to false"),
		"{message}"
	);
}

#[test]
fn a_job_with_manual_false_cannot_be_started_as_a_dependency_either() {
	// Failing closed: the closed job is not launched through another job.
	let file = parse_str(FILE).unwrap();
	let err = run_view(&file, "needs-closed")
		.err()
		.expect("needs-closed depends on a closed job");
	assert!(matches!(err, ComposeError::Job(_)), "{err:?}");
	let message = err.to_string();
	assert!(
		message.contains(
			"job 'needs-closed' depends on job 'scheduled-only', which sets `triggers.manual` to false"
		),
		"{message}"
	);
}

#[test]
fn a_closed_job_deeper_in_the_chain_is_found() {
	let yaml = "jobs:\n  a:\n    image: x\n    depends_on: [b]\n    triggers: {manual: true}\n  b:\n    image: x\n    depends_on: [c]\n    triggers: {manual: true}\n  c:\n    image: x\n    triggers: {manual: false}\n";
	let file = parse_str(yaml).unwrap();
	let message = run_view(&file, "a").err().expect("c is closed").to_string();
	assert!(message.contains("job 'b' depends on job 'c'"), "{message}");
}

#[test]
fn an_unknown_job_is_not_found() {
	let file = parse_str(FILE).unwrap();
	let err = run_view(&file, "ghost").err().expect("no such job");
	assert!(matches!(err, ComposeError::ServiceNotFound(_)), "{err:?}");
}

#[test]
fn a_service_name_is_not_a_job() {
	let file = parse_str(FILE).unwrap();
	assert!(run_view(&file, "web").is_err());
}

#[test]
fn all_jobs_can_be_listed_as_services() {
	let file = parse_str(FILE).unwrap();
	let view = with_jobs_as_services(&file);
	assert_eq!(view.services.len(), 7);
	assert!(view.jobs.is_empty());
	assert_eq!(view.services["migrate"].image.as_deref(), Some("alpine"));
}
