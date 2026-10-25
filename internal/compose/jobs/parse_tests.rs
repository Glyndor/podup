//! Typing the `jobs:` section: what a job needs, what is refused, and that an
//! error names the job without echoing the file's own text.

use crate::compose::parse_str;
use crate::compose::types::ComposeFile;
use crate::error::ComposeError;

/// One service and one manual job, the smallest shape the section takes.
const ONE_JOB: &str = "\
services:
  web:
    image: nginx
jobs:
  backup:
    image: alpine
    command: [tar, czf, /out/b.tgz, /data]
    triggers:
      manual: true
";

/// The message of the error `yaml` must produce.
fn refusal(yaml: &str) -> String {
	let err = parse_str(yaml).expect_err("this jobs section must be refused");
	assert!(
		matches!(err, ComposeError::Job(_)),
		"a bad job is a job error, got {err:?}"
	);
	err.to_string()
}

/// `yaml` with the `triggers:` block of a one-job file replaced.
fn with_triggers(block: &str) -> String {
	format!("jobs:\n  backup:\n    image: alpine\n    triggers:{block}\n")
}

#[test]
fn a_manual_job_parses_into_a_service_and_triggers() {
	let file = parse_str(ONE_JOB).expect("a manual job is valid");
	let job = &file.jobs["backup"];
	assert_eq!(job.service.image.as_deref(), Some("alpine"));
	assert!(job.service.command.is_some(), "container keys are kept");
	assert_eq!(job.triggers.manual, Some(true));
	assert!(job.triggers.schedule.is_empty());
	assert_eq!(file.services.len(), 1, "the service is untouched");
}

#[test]
fn a_job_is_not_a_service() {
	let file = parse_str(ONE_JOB).unwrap();
	assert!(!file.services.contains_key("backup"));
}

#[test]
fn a_job_without_triggers_is_refused_and_named() {
	let message = refusal("jobs:\n  backup:\n    image: alpine\n");
	assert!(
		message.contains("job 'backup' declares no triggers"),
		"{message}"
	);
}

#[test]
fn triggers_set_to_null_is_the_same_as_missing() {
	let message = refusal("jobs:\n  backup:\n    image: alpine\n    triggers:\n");
	assert!(
		message.contains("job 'backup' declares no triggers"),
		"{message}"
	);
}

#[test]
fn empty_triggers_declare_no_trigger() {
	let message = refusal(&with_triggers(" {}"));
	assert!(
		message.contains("job 'backup' declares no trigger"),
		"{message}"
	);
}

#[test]
fn a_schedule_with_no_entries_declares_no_trigger() {
	let message = refusal(&with_triggers("\n      schedule: []"));
	assert!(
		message.contains("job 'backup' declares no trigger"),
		"{message}"
	);
}

#[test]
fn triggers_of_the_wrong_type_are_refused_and_name_the_job() {
	for block in [" []", " [manual]", " manual", " 7", " true"] {
		let message = refusal(&with_triggers(block));
		assert!(
			message.contains("`triggers` of job 'backup' must be a mapping"),
			"triggers:{block} gave: {message}"
		);
	}
}

#[test]
fn a_job_that_is_not_a_mapping_is_refused() {
	for body in ["alpine", "[a, b]", "7"] {
		let message = refusal(&format!("jobs:\n  backup: {body}\n"));
		assert!(
			message.contains("job 'backup' must be a mapping"),
			"{message}"
		);
	}
}

#[test]
fn a_null_job_body_is_refused() {
	let message = refusal("jobs:\n  backup:\n");
	assert!(
		message.contains("job 'backup' must be a mapping"),
		"{message}"
	);
}

#[test]
fn an_empty_or_null_jobs_section_declares_nothing() {
	for yaml in [
		"services:\n  web:\n    image: nginx\njobs: {}\n",
		"services:\n  web:\n    image: nginx\njobs:\n",
		"services:\n  web:\n    image: nginx\n",
	] {
		let file = parse_str(yaml).expect("no job declared");
		assert!(file.jobs.is_empty(), "{yaml}");
		assert!(
			!file.extensions.contains_key("jobs"),
			"jobs is a key of the specification, not an unknown one"
		);
	}
}

#[test]
fn a_jobs_section_that_is_not_a_mapping_is_refused() {
	for body in ["[backup]", "backup", "7"] {
		let message = refusal(&format!("jobs: {body}\n"));
		assert!(
			message.contains("`jobs:` key must be a mapping"),
			"{message}"
		);
	}
}

#[test]
fn a_job_name_that_yaml_reads_as_a_number_is_refused() {
	let message = refusal("jobs:\n  1:\n    image: alpine\n    triggers: {manual: true}\n");
	assert!(message.contains("job name must be a string"), "{message}");
}

#[test]
fn a_hostile_job_name_is_escaped_in_the_message() {
	let message = refusal("jobs:\n  \"bad\\e[31mname\\n\":\n    image: alpine\n");
	assert!(
		!message.chars().any(char::is_control),
		"no raw control character may reach the terminal: {message:?}"
	);
	assert!(message.contains("bad\\u{1b}[31mname\\n"), "{message}");
}

#[test]
fn a_bad_container_key_names_the_job_and_does_not_echo_the_value() {
	let message = refusal(
		"jobs:\n  backup:\n    image: [SECRET-MARKER-91ac]\n    triggers: {manual: true}\n",
	);
	assert!(
		message.contains("job 'backup' is not a valid job definition"),
		"{message}"
	);
	assert!(!message.contains("SECRET-MARKER-91ac"), "{message}");
}

#[test]
fn profiles_extends_and_extensions_are_kept_on_the_service() {
	let yaml = "\
jobs:
  base:
    image: alpine
    triggers: {manual: true}
  backup:
    extends: base
    profiles: [ops]
    x-note: hello
    triggers: {manual: true}
";
	let file = parse_str(yaml).unwrap();
	let backup = &file.jobs["backup"].service;
	assert!(backup.profiles.contains(&"ops".to_string()));
	assert!(backup.unknown.contains_key("x-note"));
}

#[test]
fn variables_are_interpolated_inside_a_job() {
	let yaml = "jobs:\n  backup:\n    image: ${PODUP_JOBS_TEST_UNSET_IMAGE:-alpine:3}\n    triggers:\n      manual: true\n";
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.jobs["backup"].service.image.as_deref(),
		Some("alpine:3")
	);
}

#[test]
fn an_interpolated_boolean_keeps_its_type_in_triggers() {
	let yaml = "jobs:\n  backup:\n    image: alpine\n    triggers:\n      manual: ${PODUP_JOBS_TEST_UNSET_FLAG:-false}\n";
	let file = parse_str(yaml).expect("an interpolated true or false is still a boolean");
	assert_eq!(file.jobs["backup"].triggers.manual, Some(false));
}

#[test]
fn ten_thousand_jobs_parse() {
	let mut yaml = String::from("jobs:\n");
	for index in 0..10_000 {
		yaml.push_str(&format!(
			"  j{index}:\n    image: alpine\n    triggers: {{manual: true}}\n"
		));
	}
	let file = parse_str(&yaml).expect("a large jobs section is still a jobs section");
	assert_eq!(file.jobs.len(), 10_000);
	assert_eq!(file.jobs.keys().next().map(String::as_str), Some("j0"));
}

#[test]
fn jobs_keep_the_order_of_the_file() {
	let yaml = "jobs:\n  zeta:\n    image: a\n    triggers: {manual: true}\n  alpha:\n    image: a\n    triggers: {manual: true}\n";
	let file = parse_str(yaml).unwrap();
	let names: Vec<&str> = file.jobs.keys().map(String::as_str).collect();
	assert_eq!(names, ["zeta", "alpha"]);
}

#[test]
fn a_job_round_trips_through_serialization_with_its_triggers() {
	let yaml = "\
jobs:
  backup:
    image: alpine
    triggers:
      manual: false
      schedule:
        - '0 3 * * *'
        - cron: '*/5 * * * *'
          timezone: Europe/Berlin
          concurrency: queue
          missed_fires: skip
";
	let file = parse_str(yaml).unwrap();
	let text = serde_yaml::to_string(&file).unwrap();
	let again: ComposeFile = parse_str(&text).expect("the rendered file parses back");
	assert_eq!(again.jobs["backup"].triggers, file.jobs["backup"].triggers);
	assert_eq!(
		again.jobs["backup"].service.image.as_deref(),
		Some("alpine")
	);
}

#[test]
fn an_empty_jobs_section_is_not_rendered() {
	let file = parse_str("services:\n  web:\n    image: nginx\njobs: {}\n").unwrap();
	let text = serde_yaml::to_string(&file).unwrap();
	assert!(!text.contains("jobs"), "{text}");
}

#[test]
fn the_default_file_declares_no_job() {
	assert!(ComposeFile::default().jobs.is_empty());
}
