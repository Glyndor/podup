//! An invalid `umask` names the service and the rule, and does not repeat the
//! value. In a service the value is read after interpolation, so
//! `umask: ${DB_PASSWORD}` would print the password. A job's error already left
//! the value out; the service path now does too (docker compose repeats it, and
//! podup does not follow it there).

use super::umask::{validate, validate_runtime};

const SECRET: &str = "p@ssw0rd-not-octal";

fn text_of(error: impl std::fmt::Display) -> String {
	error.to_string()
}

#[test]
fn the_runtime_check_does_not_repeat_the_value() {
	let error = validate_runtime("web", SECRET).expect_err("not octal");
	let text = text_of(error);
	assert!(text.contains("service.web: umask:"), "{text}");
	assert!(text.contains("octal"), "{text}");
	assert!(!text.contains(SECRET) && !text.contains("(value"), "{text}");
}

#[test]
fn the_parse_check_does_not_repeat_the_value() {
	let yaml = serde_yaml::from_str(&format!(
		"services:\n  web:\n    image: x\n    umask: {SECRET}\n"
	))
	.unwrap();
	let text = text_of(validate(&yaml).expect_err("not octal"));
	assert!(text.contains("service.web: umask:"), "{text}");
	assert!(!text.contains(SECRET), "{text}");
}

#[test]
fn a_whole_number_is_refused_without_its_value_too() {
	let error = crate::parse_str(
		"services:\n  web:\n    image: x\n    umask: 123456789012345678901234567890\n",
	)
	.expect_err("a number is not a quoted octal string");
	let text = text_of(error);
	assert!(text.contains("umask"), "{text}");
	assert!(!text.contains("123456789012345678901234567890"), "{text}");
}

#[test]
fn other_fields_still_show_the_value_they_were_given() {
	// The namespace validators keep `(value: ...)`: it is the diagnostic there.
	let error = crate::libpod::validate::spec_field_error("web", "pid", "hots", "bad mode");
	assert!(error.to_string().contains("(value: hots)"), "{error}");
}

#[test]
fn a_control_character_in_the_service_name_does_not_reach_the_message() {
	let text = text_of(validate_runtime("we\u{1b}[31mb", SECRET).expect_err("not octal"));
	assert!(!text.contains('\u{1b}'), "{text:?}");
}
