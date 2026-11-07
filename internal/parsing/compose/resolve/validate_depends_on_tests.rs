//! `depends_on` must name a service the file defines, required or not.

use super::*;
use crate::parse_str;

fn validate_str(yaml: &str) -> crate::error::Result<()> {
	let mut file: ComposeFile = parse_str(yaml).unwrap();
	crate::compose::normalize_default_network(&mut file);
	super::validate(&file)
}

fn message(yaml: &str) -> String {
	validate_str(yaml)
		.expect_err("an undefined dependency must be refused")
		.to_string()
}

#[test]
fn an_undefined_dependency_names_the_service_and_the_dependency() {
	let text = message("services:\n  app:\n    image: x\n    depends_on: [nope]\n");
	assert!(text.contains("'app'") && text.contains("'nope'"), "{text}");
	assert!(text.contains("undefined service"), "{text}");
	assert!(
		!text.contains("unsupported feature"),
		"an input error, not a feature: {text}"
	);
}

#[test]
fn required_false_does_not_excuse_a_dependency_defined_nowhere() {
	let text = message(
		"services:\n  app:\n    image: x\n    depends_on:\n      nope:\n        condition: service_started\n        required: false\n",
	);
	assert!(text.contains("'app'") && text.contains("'nope'"), "{text}");
}

#[test]
fn a_defined_dependency_with_required_false_passes() {
	validate_str(
		"services:\n  app:\n    image: x\n    depends_on:\n      db:\n        condition: service_started\n        required: false\n  db:\n    image: x\n",
	)
	.expect("db is defined");
}

#[test]
fn a_dependency_in_an_inactive_profile_is_defined_and_passes() {
	// The profile only decides whether it runs; the owner's ruling on #1276
	// keeps that case as it is.
	validate_str(
		"services:\n  app:\n    image: x\n    depends_on: [db]\n  db:\n    image: x\n    profiles: [debug]\n",
	)
	.expect("db is defined");
}

#[test]
fn a_job_depending_on_something_undefined_is_refused_here_and_not_only_by_config() {
	// `validate` runs in every command; `config` alone used to reject this file,
	// and `up` created the project.
	let text = message(
		"services:\n  web:\n    image: x\njobs:\n  backup:\n    image: x\n    triggers: {manual: true}\n    depends_on:\n      ghost:\n        condition: service_started\n        required: false\n",
	);
	assert!(
		text.contains("'backup'") && text.contains("'ghost'"),
		"{text}"
	);
}

#[test]
fn a_job_depending_on_a_service_or_another_job_passes() {
	validate_str(
		"services:\n  web:\n    image: x\njobs:\n  a:\n    image: x\n    triggers: {manual: true}\n    depends_on: [web, b]\n  b:\n    image: x\n    triggers: {manual: true}\n",
	)
	.expect("both are defined");
}

#[test]
fn a_control_character_in_a_dependency_name_does_not_reach_the_message() {
	let text = message("services:\n  app:\n    image: x\n    depends_on: [\"no\\e[31mpe\"]\n");
	assert!(!text.contains('\u{1b}'), "{text:?}");
	assert!(text.contains("\\u{1b}"), "{text:?}");
}
