//! Messages about a job say "job"; messages about a service are left alone.

use super::{relabel, relabel_error};
use crate::compose::parse_str;
use crate::compose::validate_config;
use crate::error::ComposeError;

const FILE: &str = "\
services:
  web:
    image: x
jobs:
  migrate:
    image: x
    triggers: {manual: true}
";

#[test]
fn a_warning_about_a_job_says_job_in_both_quote_styles() {
	let file = parse_str(FILE).unwrap();
	assert_eq!(
		relabel(&file, "service 'migrate': unknown key".to_string()),
		"job 'migrate': unknown key"
	);
	assert_eq!(
		relabel(&file, "service \"migrate\": privileged".to_string()),
		"job \"migrate\": privileged"
	);
	assert_eq!(
		relabel(&file, "service 'migrate/pre_start[0]': x".to_string()),
		"job 'migrate/pre_start[0]': x"
	);
}

#[test]
fn a_warning_about_a_service_is_unchanged() {
	let file = parse_str(FILE).unwrap();
	for message in [
		"service 'web': unknown key",
		"service \"web\": privileged",
		"unknown top-level key",
	] {
		assert_eq!(relabel(&file, message.to_string()), message);
	}
}

#[test]
fn validation_errors_about_a_job_say_job() {
	for (yaml, expected) in [
		(
			"jobs:\n  a:\n    image: x\n    networks: [ghost]\n    triggers: {manual: true}\n",
			"job 'a' refers to undefined network 'ghost'",
		),
		(
			"jobs:\n  a:\n    image: x\n    volumes: ['ghost:/d']\n    triggers: {manual: true}\n",
			"job 'a' refers to undefined volume 'ghost'",
		),
		(
			"jobs:\n  'a b':\n    image: x\n    triggers: {manual: true}\n",
			"job name \"a b\" is invalid",
		),
		(
			"jobs:\n  a:\n    triggers: {manual: true}\n",
			"job 'a' has neither an image nor a build",
		),
		(
			"jobs:\n  a:\n    image: x\n    depends_on: [ghost]\n    triggers: {manual: true}\n",
			"job 'a' depends on 'ghost', which is not defined as a service or a job",
		),
	] {
		let file = parse_str(yaml).unwrap();
		let message = validate_config(&file).expect_err(yaml).to_string();
		assert!(message.contains(expected), "{expected}: {message}");
		assert!(!message.contains("service '"), "{message}");
	}
}

#[test]
fn an_optional_dependency_that_is_missing_is_not_an_error() {
	let yaml = "jobs:\n  a:\n    image: x\n    depends_on:\n      ghost:\n        condition: service_started\n        required: false\n    triggers: {manual: true}\n";
	validate_config(&parse_str(yaml).unwrap()).expect("optional dependency");
}

#[test]
fn other_errors_pass_through_untouched() {
	let error = relabel_error("a", ComposeError::Interrupted);
	assert!(matches!(error, ComposeError::Interrupted));
}
