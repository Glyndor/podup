//! Compose mask parsing and scalar inheritance.

use crate::compose::{parse_str, parse_str_raw};

fn mask(value: &str) -> super::Service {
	parse_str_raw(&format!(
		"services:\n  web:\n    image: alpine\n    umask: {value}\n"
	))
	.unwrap()
	.services
	.swap_remove("web")
	.unwrap()
}

#[test]
fn umask_zero_and_leading_zero_strings_keep_text() {
	for (value, expected) in [("0", "0"), ("022", "022"), ("0022", "0022")] {
		let service = mask(value);
		assert_eq!(service.umask.as_deref(), Some(expected));
		assert!(!service.unknown.contains_key("umask"));
	}
}

#[test]
fn umask_nonzero_integer_requires_quotes() {
	for value in [
		"22",
		"77",
		"18",
		"0o30",
		"0x16",
		"0x18",
		"0b10110",
		"4294967295",
	] {
		let error = parse_str_raw(&format!("services:\n  worker:\n    umask: {value}\n"))
			.unwrap_err()
			.to_string();
		assert!(
			error.contains("worker") && error.contains("quote"),
			"{value}: {error}"
		);
	}
}

#[test]
fn umask_string_four_digits() {
	assert_eq!(mask("\"0022\"").umask, Some("0022".into()));
}

#[test]
fn umask_string_three_digits() {
	assert_eq!(mask("\"022\"").umask, Some("022".into()));
}

#[test]
fn umask_string_two_digits() {
	assert_eq!(mask("\"22\"").umask, Some("22".into()));
	assert_eq!(mask("\"7777\"").umask.as_deref(), Some("7777"));
}

#[test]
fn umask_null_and_omitted_are_unset() {
	assert_eq!(mask("null").umask, None);
	let file = parse_str_raw("services:\n  web:\n    image: alpine\n").unwrap();
	assert_eq!(file.services["web"].umask, None);
}

#[test]
fn umask_invalid_names_service() {
	for value in [
		"\"0088\"", "\"word\"", "\"\"", "\"+22\"", "-1", "true", "0.0", "1.5", "[]", "{}",
	] {
		let error =
			parse_str_raw(&format!("services:\n  worker:\n    umask: {value}\n")).unwrap_err();
		match error {
			crate::error::ComposeError::Podman(crate::libpod::PodmanError::Field {
				service,
				field,
				..
			}) => {
				assert_eq!(service, "worker");
				assert_eq!(field, "umask");
			}
			other => panic!("expected service-aware field error, got {other}"),
		}
	}
}

#[test]
fn umask_out_of_range_names_service() {
	for value in ["\"10000\"", "\"37777777777\"", "\"00000\""] {
		let error = parse_str_raw(&format!("services:\n  worker:\n    umask: {value}\n"))
			.unwrap_err()
			.to_string();
		assert!(error.contains("worker"), "{error}");
		assert!(error.contains("umask"), "{error}");
	}
}

#[test]
fn umask_extends_inherits_scalar() {
	let file =
		parse_str("services:\n  base:\n    umask: '0022'\n  web:\n    extends: base\n").unwrap();
	assert_eq!(file.services["web"].umask, Some("0022".into()));
}

#[test]
fn umask_extends_zero_overrides_scalar() {
	let file = parse_str(
		"services:\n  base:\n    umask: '0022'\n  web:\n    extends: base\n    umask: 0\n",
	)
	.unwrap();
	assert_eq!(file.services["web"].umask, Some("0".into()));
}
