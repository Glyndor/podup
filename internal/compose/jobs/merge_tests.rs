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
