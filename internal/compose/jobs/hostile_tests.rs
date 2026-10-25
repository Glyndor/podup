//! Documents that lie about their shape: a job or a service that is not a
//! mapping, with and without the merge tags. A malformed document is an error,
//! never a panic: the release profile aborts on one.

use crate::compose::parse_str;
use crate::compose::tags;
use crate::compose::types::service::service_umask;

/// A service named `same` that asks for `umask` to be reset, beside a `same`
/// job of the given (wrong) shape.
fn services_reset_beside_job(job: &str) -> String {
	format!("services:\n  same:\n    image: alpine\n    umask: !reset null\njobs:\n  same: {job}\n")
}

/// A job named `same` that asks for `umask` to be reset, beside a `same`
/// service of the given (wrong) shape.
fn job_reset_beside_service(service: &str) -> String {
	format!(
		"services:\n  same: {service}\njobs:\n  same:\n    image: alpine\n    umask: !reset null\n    triggers: {{manual: true}}\n"
	)
}

#[test]
fn a_reset_in_services_does_not_reach_a_job_of_the_same_name() {
	for shape in ["false", "[a, b]", "7", "text", "null", "!reset false"] {
		let result = parse_str(&services_reset_beside_job(shape));
		assert!(result.is_err(), "{shape}: a malformed job is an error");
	}
}

#[test]
fn a_reset_in_a_job_does_not_reach_a_service_of_the_same_name() {
	for shape in ["false", "[a, b]", "7", "text", "null"] {
		let result = parse_str(&job_reset_beside_service(shape));
		assert!(result.is_err(), "{shape}: a malformed service is an error");
	}
}

#[test]
fn override_beside_a_malformed_job_is_an_error_too() {
	for shape in ["false", "[a]", "7", "null"] {
		let yaml = format!(
			"services:\n  same:\n    image: alpine\n    environment: !override {{A: b}}\njobs:\n  same: {shape}\n"
		);
		assert!(parse_str(&yaml).is_err(), "{shape}");
	}
}

#[test]
fn every_job_shape_that_is_not_a_mapping_is_an_error_naming_the_job() {
	for shape in [
		"false",
		"[a, b]",
		"7",
		"text",
		"null",
		"!reset null",
		"!override []",
	] {
		let yaml = format!("jobs:\n  odd: {shape}\n");
		let message = parse_str(&yaml).expect_err(shape).to_string();
		assert!(message.contains("job 'odd'"), "{shape}: {message}");
	}
}

#[test]
fn the_reset_of_a_umask_lands_in_the_section_that_asked_for_it() {
	let yaml = "services:\n  web:\n    image: alpine\n    umask: '0022'\n  other:\n    image: alpine\n    umask: !reset null\njobs:\n  backup:\n    image: alpine\n    umask: !reset null\n    triggers: {manual: true}\n  keeper:\n    image: alpine\n    umask: '0077'\n    triggers: {manual: true}\n";
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["web"].umask.as_deref(), Some("0022"));
	assert_eq!(file.services["other"].umask, None);
	assert_eq!(file.jobs["backup"].service.umask, None);
	assert_eq!(file.jobs["keeper"].service.umask.as_deref(), Some("0077"));
}

#[test]
fn resets_are_found_by_section_and_name() {
	let raw: serde_yaml::Value = serde_yaml::from_str(
		"services:\n  a:\n    umask: !reset null\n  b: false\njobs:\n  a: [x]\n  c:\n    umask: !reset null\n  d: 7\n",
	)
	.unwrap();
	let mut found = tags::umask_resets(&raw);
	found.sort();
	assert_eq!(
		found,
		[("jobs", "c".to_string()), ("services", "a".to_string())]
	);
}

#[test]
fn applying_resets_to_a_document_of_the_wrong_shape_changes_nothing() {
	for text in ["false", "[1, 2]", "7", "null", "\"s\""] {
		let mut value: serde_yaml::Value =
			serde_yaml::from_str(&format!("services:\n  a: {text}\njobs:\n  a: {text}\n")).unwrap();
		let before = value.clone();
		service_umask::apply_resets(
			&mut value,
			&[("services", "a".to_string()), ("jobs", "a".to_string())],
		);
		assert_eq!(value, before, "{text}");
		let mut scalar: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
		service_umask::apply_resets(&mut scalar, &[("jobs", "a".to_string())]);
	}
}
