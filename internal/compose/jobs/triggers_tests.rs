//! The `triggers:` block: shape of `manual` and `schedule`, and the limits that
//! keep hostile text out of what a later step writes into other files.

use super::{MAX_CRON_BYTES, MAX_SCHEDULES, MAX_TIMEZONE_BYTES};
use crate::compose::parse_str;
use crate::compose::types::{Concurrency, MissedFires, Schedule};
use crate::error::ComposeError;

/// A one-job file whose `triggers:` block is `block`, indented under it.
fn job(block: &str) -> String {
	let indented: String = block.lines().map(|l| format!("      {l}\n")).collect();
	format!("jobs:\n  backup:\n    image: alpine\n    triggers:\n{indented}")
}

fn refusal(block: &str) -> String {
	let err = parse_str(&job(block)).expect_err("these triggers must be refused");
	assert!(matches!(err, ComposeError::Job(_)), "{err:?}");
	err.to_string()
}

fn schedule_of(block: &str) -> Vec<Schedule> {
	parse_str(&job(block)).unwrap().jobs["backup"]
		.triggers
		.schedule
		.clone()
}

#[test]
fn manual_defaults_to_allowed() {
	let file = parse_str(&job("schedule: ['0 3 * * *']")).unwrap();
	let triggers = &file.jobs["backup"].triggers;
	assert_eq!(triggers.manual, None);
	assert!(triggers.allows_manual());
}

#[test]
fn manual_false_alone_is_a_declared_trigger() {
	let file = parse_str(&job("manual: false")).unwrap();
	assert!(!file.jobs["backup"].triggers.allows_manual());
}

#[test]
fn manual_must_be_a_boolean() {
	for value in ["'yes'", "'False'", "'1'", "0", "[]", "{}"] {
		let message = refusal(&format!("manual: {value}"));
		assert!(
			message.contains("`triggers.manual` of job 'backup' must be true or false"),
			"manual: {value} gave: {message}"
		);
	}
}

#[test]
fn schedule_must_be_a_list() {
	for value in ["'0 3 * * *'", "7", "{cron: '* * * * *'}", "true"] {
		let message = refusal(&format!("schedule: {value}"));
		assert!(
			message.contains("`triggers.schedule` of job 'backup' must be a list"),
			"schedule: {value} gave: {message}"
		);
	}
}

#[test]
fn an_unknown_trigger_type_is_refused() {
	let message = refusal("manual: true\nwebhook: {url: x}");
	assert!(message.contains("unknown trigger type"), "{message}");
	assert!(
		!message.contains("webhook"),
		"the key is the file's text: {message}"
	);
}

#[test]
fn a_schedule_entry_is_a_string_or_a_mapping() {
	for entry in ["7", "[a]", "null", "true"] {
		let message = refusal(&format!("schedule: [{entry}]"));
		assert!(
			message.contains("schedule entry 1 of job 'backup' must be a crontab string"),
			"[{entry}] gave: {message}"
		);
	}
}

#[test]
fn the_string_and_mapping_forms_both_parse() {
	let block = "schedule:\n  - '0 3 * * *'\n  - cron: '*/5 * * * *'\n    timezone: America/Argentina/Buenos_Aires\n    concurrency: queue\n    missed_fires: skip\n  - cron: '@daily'";
	let entries = schedule_of(block);
	assert_eq!(entries.len(), 3);
	assert_eq!(entries[0].cron(), "0 3 * * *");
	let Schedule::Detailed(spec) = &entries[1] else {
		panic!("the second entry is the object form");
	};
	assert_eq!(
		spec.timezone.as_deref(),
		Some("America/Argentina/Buenos_Aires")
	);
	assert_eq!(spec.concurrency, Some(Concurrency::Queue));
	assert_eq!(spec.missed_fires, Some(MissedFires::Skip));
	let Schedule::Detailed(bare) = &entries[2] else {
		panic!("the third entry is the object form");
	};
	assert_eq!((bare.concurrency, bare.missed_fires), (None, None));
}

#[test]
fn every_valid_policy_value_is_accepted() {
	for (key, value) in [
		("concurrency", "forbid"),
		("concurrency", "queue"),
		("missed_fires", "one"),
		("missed_fires", "skip"),
	] {
		let block = format!("schedule:\n  - cron: '* * * * *'\n    {key}: {value}");
		assert_eq!(schedule_of(&block).len(), 1, "{key}: {value}");
	}
}

#[test]
fn an_invalid_concurrency_is_refused() {
	for value in ["allow", "Forbid", "''", "1", "null", "[forbid]"] {
		let message = refusal(&format!(
			"schedule:\n  - cron: '* * * * *'\n    concurrency: {value}"
		));
		assert!(
			message.contains("`concurrency` in schedule entry 1 of job 'backup'"),
			"concurrency: {value} gave: {message}"
		);
	}
}

#[test]
fn an_invalid_missed_fires_is_refused() {
	for value in ["all", "ONE", "''", "0", "null"] {
		let message = refusal(&format!(
			"schedule:\n  - cron: '* * * * *'\n    missed_fires: {value}"
		));
		assert!(
			message.contains("`missed_fires` in schedule entry 1 of job 'backup'"),
			"missed_fires: {value} gave: {message}"
		);
	}
}

#[test]
fn an_unknown_key_in_a_schedule_entry_is_refused() {
	let message = refusal("schedule:\n  - cron: '* * * * *'\n    retries: 3");
	assert!(
		message.contains("schedule entry 1 of job 'backup' has an unknown key"),
		"{message}"
	);
}

#[test]
fn a_schedule_entry_needs_a_cron() {
	let message = refusal("schedule:\n  - timezone: UTC");
	assert!(message.contains("needs a `cron` expression"), "{message}");
}

#[test]
fn cron_must_be_a_string() {
	for value in ["7", "[a]", "null", "true"] {
		let message = refusal(&format!("schedule:\n  - cron: {value}"));
		assert!(
			message.contains("`cron` in schedule entry 1 of job 'backup' must be a string"),
			"cron: {value} gave: {message}"
		);
	}
}

#[test]
fn a_blank_cron_is_refused() {
	for value in ["''", "'   '"] {
		let message = refusal(&format!("schedule: [{value}]"));
		assert!(
			message.contains("has an empty cron expression"),
			"{message}"
		);
		let message = refusal(&format!("schedule:\n  - cron: {value}"));
		assert!(
			message.contains("has an empty cron expression"),
			"{message}"
		);
	}
}

#[test]
fn a_cron_with_a_control_character_is_refused_and_not_echoed() {
	// A newline would start a second directive in a unit file; ESC would reach
	// a terminal. Neither the character nor the text around it may come back.
	for cron in [
		"0 3 * * *\\nExecStart=/bin/sh",
		"0 3 * * *\\u001b[31m",
		"0\\t3 * * *",
		"0 3 * * *\\u007f",
		"0 3 * * *\\u2028ExecStart=/bin/sh",
		"0 3 * * *\\u202e",
		"0\\u00a03 * * *",
		"0 3 * * *; reboot",
		"$(id) * * * *",
		"0 3 * * * `id`",
	] {
		let message = refusal(&format!("schedule:\n  - \"{cron}\""));
		assert!(
			message.contains("has a character outside"),
			"{cron}: {message}"
		);
		assert!(!message.contains("ExecStart"), "{message}");
		assert!(!message.chars().any(char::is_control), "{message:?}");
	}
}

#[test]
fn the_cron_length_cap_is_exact() {
	let at_cap = "5".repeat(MAX_CRON_BYTES);
	assert_eq!(schedule_of(&format!("schedule: ['{at_cap}']")).len(), 1);
	let over = "5".repeat(MAX_CRON_BYTES + 1);
	let message = refusal(&format!("schedule: ['{over}']"));
	assert!(message.contains("is longer than"), "{message}");
	assert!(!message.contains(&over), "the oversized text is not echoed");
}

#[test]
fn a_gigantic_cron_is_refused_without_being_stored() {
	let huge = "9".repeat(2 * 1024 * 1024);
	let message = refusal(&format!("schedule:\n  - cron: '{huge}'"));
	assert!(message.contains("is longer than"), "{message}");
	assert!(
		message.len() < 512,
		"the message stays small: {} bytes",
		message.len()
	);
}

#[test]
fn a_cron_is_stored_trimmed() {
	let entries = schedule_of("schedule: ['  0 3 * * *  ']");
	assert_eq!(entries[0].cron(), "0 3 * * *");
}

#[test]
fn a_timezone_is_a_bounded_name() {
	for zone in [
		"Europe/Berlin",
		"UTC",
		"Etc/GMT+5",
		"America/Port-au-Prince",
	] {
		let block = format!("schedule:\n  - cron: '* * * * *'\n    timezone: {zone}");
		assert_eq!(schedule_of(&block).len(), 1, "{zone}");
	}
	let long = format!("'{}'", "Z".repeat(MAX_TIMEZONE_BYTES + 1));
	let bad = [
		"''",
		"\"Europe/Berlin\\n\"",
		"'a b'",
		"'../etc'",
		"'/etc/localtime'",
		"7",
		"null",
		long.as_str(),
	];
	for zone in bad {
		let block = format!("schedule:\n  - cron: '* * * * *'\n    timezone: {zone}");
		let message = refusal(&block);
		assert!(
			message.contains("`timezone`"),
			"timezone {zone} gave: {message}"
		);
	}
}

#[test]
fn the_schedule_count_cap_is_exact() {
	let entries = |count: usize| (0..count).map(|_| "- '* * * * *'\n").collect::<String>();
	assert_eq!(
		schedule_of(&format!("schedule:\n{}", entries(MAX_SCHEDULES))).len(),
		MAX_SCHEDULES
	);
	let message = refusal(&format!("schedule:\n{}", entries(MAX_SCHEDULES + 1)));
	assert!(message.contains("has more than"), "{message}");
}

#[test]
fn a_schedule_error_names_the_entry_by_position() {
	let message = refusal("schedule:\n  - '* * * * *'\n  - '* * * * *'\n  - ''");
	assert!(
		message.contains("schedule entry 3 of job 'backup'"),
		"{message}"
	);
}

#[test]
fn manual_and_an_empty_schedule_together_are_valid() {
	let file = parse_str(&job("manual: true\nschedule: []")).unwrap();
	assert_eq!(file.jobs["backup"].triggers.manual, Some(true));
}

#[test]
fn manual_accepts_the_text_true_and_false() {
	// `podup config` prints a boolean as quoted text; its output must read back.
	for (value, expected) in [
		("'true'", true),
		("'false'", false),
		("true", true),
		("false", false),
	] {
		let file = parse_str(&job(&format!("manual: {value}"))).unwrap();
		assert_eq!(
			file.jobs["backup"].triggers.manual,
			Some(expected),
			"{value}"
		);
	}
}

#[test]
fn a_crontab_expression_and_its_macros_pass_the_whitelist() {
	for cron in [
		"0 3 * * *",
		"*/5 0-6,22 1-15 jan,feb mon-fri",
		"@daily",
		"@every 5m",
		"0 0 ? * 5#2",
		"0 0 L * *",
	] {
		assert_eq!(
			schedule_of(&format!("schedule: ['{cron}']")).len(),
			1,
			"{cron}"
		);
	}
}

#[test]
fn a_null_schedule_is_an_absent_one() {
	let file = parse_str(&job("manual: true\nschedule:")).unwrap();
	assert!(file.jobs["backup"].triggers.schedule.is_empty());
	let message = refusal("schedule:");
	assert!(message.contains("declares no triggers"), "{message}");
}
