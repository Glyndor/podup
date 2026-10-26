//! A job declared in more than one `-f` file.

use crate::compose::parse_files_with_env_files;
use crate::compose::types::ComposeFile;

/// Parse `files` as `-f a -f b ...` from a temp dir.
fn merged(files: &[&str]) -> ComposeFile {
	let dir = tempfile::tempdir().unwrap();
	let paths: Vec<_> = files
		.iter()
		.enumerate()
		.map(|(index, text)| {
			let path = dir.path().join(format!("f{index}.yaml"));
			std::fs::write(&path, text).unwrap();
			path
		})
		.collect();
	parse_files_with_env_files(&paths, &[]).expect("the files merge")
}

const BASE: &str = "\
jobs:
  backup:
    image: alpine:3
    environment: {A: '1'}
    ports: ['8080:80']
    triggers:
      manual: true
      schedule: ['0 3 * * *']
";

#[test]
fn a_later_file_changes_one_key_and_keeps_the_rest() {
	let file = merged(&[
		BASE,
		"jobs:\n  backup:\n    image: alpine:4\n    triggers: {manual: true}\n",
	]);
	let job = &file.jobs["backup"];
	assert_eq!(job.service.image.as_deref(), Some("alpine:4"));
	assert!(
		job.service.environment.to_map().contains_key("A"),
		"environment is kept"
	);
}

#[test]
fn a_later_file_adds_a_job() {
	let file = merged(&[
		BASE,
		"jobs:\n  other:\n    image: x\n    triggers: {manual: true}\n",
	]);
	assert_eq!(file.jobs.len(), 2);
}

#[test]
fn a_later_schedule_replaces_the_earlier_list() {
	let over =
		"jobs:\n  backup:\n    image: alpine:3\n    triggers:\n      schedule: ['*/10 * * * *']\n";
	let job = &merged(&[BASE, over]).jobs["backup"];
	let crons: Vec<&str> = job.triggers.schedule.iter().map(|s| s.cron()).collect();
	assert_eq!(crons, ["*/10 * * * *"], "replaced, not appended");
	assert_eq!(
		job.triggers.manual,
		Some(true),
		"manual is kept when not restated"
	);
}

#[test]
fn a_later_file_can_close_manual_launching() {
	let over = "jobs:\n  backup:\n    image: alpine:3\n    triggers: {manual: false}\n";
	let job = &merged(&[BASE, over]).jobs["backup"];
	assert!(!job.triggers.allows_manual());
	assert_eq!(job.triggers.schedule.len(), 1, "the schedule is kept");
}

#[test]
fn reset_and_override_tags_reach_a_job() {
	let reset = "jobs:\n  backup:\n    image: alpine:3\n    ports: !reset []\n    triggers: {manual: true}\n";
	let job = &merged(&[BASE, reset]).jobs["backup"];
	assert!(
		job.service.ports.is_empty(),
		"!reset drops the port of the first file"
	);

	let kept = "jobs:\n  backup:\n    image: alpine:3\n    ports: ['9090:90']\n    triggers: {manual: true}\n";
	let job = &merged(&[BASE, kept]).jobs["backup"];
	assert_eq!(job.service.ports.len(), 2, "without a tag ports append");

	let overridden = "jobs:\n  backup:\n    image: alpine:3\n    ports: !override ['9090:90']\n    triggers: {manual: true}\n";
	let job = &merged(&[BASE, overridden]).jobs["backup"];
	assert_eq!(
		job.service.ports.len(),
		1,
		"!override takes the later list whole"
	);
}

/// The error `files` must produce when merged.
fn merge_error(files: &[&str]) -> String {
	let dir = tempfile::tempdir().unwrap();
	let paths: Vec<_> = files
		.iter()
		.enumerate()
		.map(|(index, text)| {
			let path = dir.path().join(format!("f{index}.yaml"));
			std::fs::write(&path, text).unwrap();
			path
		})
		.collect();
	parse_files_with_env_files(&paths, &[])
		.expect_err("this merge must be refused")
		.to_string()
}

#[test]
fn an_override_may_leave_the_triggers_to_the_first_file() {
	let over = "jobs:\n  backup:\n    image: alpine:9\n";
	let job = &merged(&[BASE, over]).jobs["backup"];
	assert_eq!(job.service.image.as_deref(), Some("alpine:9"));
	assert_eq!(
		job.triggers.schedule.len(),
		1,
		"the triggers of the first file stay"
	);
}

#[test]
fn a_job_with_no_triggers_in_any_file_is_refused_after_the_merge() {
	let first = "jobs:\n  backup:\n    image: alpine\n";
	let second = "jobs:\n  backup:\n    image: alpine:2\n";
	let message = merge_error(&[first, second]);
	assert!(
		message.contains("job 'backup' declares no triggers"),
		"{message}"
	);
}

#[test]
fn a_single_file_job_without_triggers_is_still_refused() {
	let message = merge_error(&["jobs:\n  backup:\n    image: alpine\n"]);
	assert!(
		message.contains("job 'backup' declares no triggers"),
		"{message}"
	);
}

#[test]
fn reset_on_the_schedule_empties_it_and_keeps_manual() {
	let over = "jobs:\n  backup:\n    triggers:\n      schedule: !reset []\n";
	let job = &merged(&[BASE, over]).jobs["backup"];
	assert!(job.triggers.schedule.is_empty(), "{:?}", job.triggers);
	assert_eq!(job.triggers.manual, Some(true));
}

#[test]
fn reset_on_the_schedule_of_a_job_with_nothing_else_is_refused() {
	let first =
		"jobs:\n  backup:\n    image: alpine\n    triggers:\n      schedule: ['0 3 * * *']\n";
	let over = "jobs:\n  backup:\n    triggers:\n      schedule: !reset []\n";
	let message = merge_error(&[first, over]);
	assert!(
		message.contains("job 'backup' declares no triggers"),
		"{message}"
	);
}

#[test]
fn override_on_the_schedule_takes_the_later_list_whole() {
	let over = "jobs:\n  backup:\n    triggers:\n      schedule: !override ['*/10 * * * *']\n";
	let job = &merged(&[BASE, over]).jobs["backup"];
	let crons: Vec<&str> = job.triggers.schedule.iter().map(|s| s.cron()).collect();
	assert_eq!(crons, ["*/10 * * * *"]);
}

#[test]
fn override_on_triggers_replaces_the_whole_block() {
	let over = "jobs:\n  backup:\n    triggers: !override\n      manual: false\n";
	let job = &merged(&[BASE, over]).jobs["backup"];
	assert!(!job.triggers.allows_manual());
	assert!(
		job.triggers.schedule.is_empty(),
		"the earlier schedule is gone"
	);
}

#[test]
fn reset_on_triggers_leaves_the_job_without_any() {
	let over = "jobs:\n  backup:\n    triggers: !reset {}\n";
	let message = merge_error(&[BASE, over]);
	assert!(
		message.contains("job 'backup' declares no triggers"),
		"{message}"
	);
}

#[test]
fn reset_on_manual_clears_it() {
	let over = "jobs:\n  backup:\n    triggers:\n      manual: !reset null\n";
	let job = &merged(&[BASE, over]).jobs["backup"];
	assert_eq!(job.triggers.manual, None);
}
