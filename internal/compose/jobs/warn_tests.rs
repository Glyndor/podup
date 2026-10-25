//! The notice a job gets when podup cannot satisfy its triggers by itself.

use super::unscheduled_messages;
use crate::compose::parse_str;

fn messages(yaml: &str) -> Vec<String> {
	let file = parse_str(yaml).unwrap();
	unscheduled_messages(&file.jobs)
}

#[test]
fn a_job_with_three_schedule_entries_is_reported_once() {
	let yaml = "jobs:\n  nightly:\n    image: alpine\n    triggers:\n      schedule: ['0 1 * * *', '0 2 * * *', {cron: '0 3 * * *'}]\n";
	let got = messages(yaml);
	assert_eq!(got.len(), 1, "one notice per job, not per trigger: {got:?}");
	assert!(got[0].contains("job 'nightly'"), "{}", got[0]);
	assert!(got[0].contains("3 entries"), "{}", got[0]);
	assert!(got[0].contains("podup run nightly"), "{}", got[0]);
	assert!(
		got[0].contains("exports it as a systemd timer"),
		"{}",
		got[0]
	);
}

#[test]
fn every_job_with_a_schedule_is_reported() {
	let yaml = "jobs:\n  a:\n    image: x\n    triggers: {schedule: ['* * * * *']}\n  b:\n    image: x\n    triggers: {schedule: ['* * * * *']}\n";
	assert_eq!(messages(yaml).len(), 2);
}

#[test]
fn a_manual_only_job_is_not_reported() {
	let yaml = "jobs:\n  a:\n    image: x\n    triggers: {manual: true}\n";
	assert!(messages(yaml).is_empty());
}

#[test]
fn manual_false_without_a_schedule_says_nothing_can_start_it() {
	let yaml = "jobs:\n  a:\n    image: x\n    triggers: {manual: false}\n";
	let got = messages(yaml);
	assert_eq!(got.len(), 1);
	assert!(got[0].contains("nothing can start it"), "{}", got[0]);
}

#[test]
fn manual_false_with_a_schedule_does_not_offer_podup_run() {
	let yaml =
		"jobs:\n  a:\n    image: x\n    triggers: {manual: false, schedule: ['* * * * *']}\n";
	let got = messages(yaml);
	assert_eq!(got.len(), 1);
	assert!(!got[0].contains("podup run"), "{}", got[0]);
	assert!(got[0].contains("nothing starts this job"), "{}", got[0]);
}

#[test]
fn the_job_name_is_escaped() {
	let yaml = "jobs:\n  \"a\\u001bb\":\n    image: x\n    triggers: {schedule: ['* * * * *']}\n";
	let got = messages(yaml);
	assert!(!got[0].chars().any(char::is_control), "{:?}", got[0]);
}

#[test]
fn a_file_without_jobs_has_nothing_to_report() {
	assert!(messages("services:\n  web:\n    image: x\n").is_empty());
}

#[test]
fn one_entry_is_not_called_entries() {
	let yaml = "jobs:\n  a:\n    image: x\n    triggers: {schedule: ['* * * * *']}\n";
	let got = messages(yaml);
	assert!(got[0].contains("(1 entry)"), "{}", got[0]);
	assert!(!got[0].contains("1 entries"), "{}", got[0]);
}
