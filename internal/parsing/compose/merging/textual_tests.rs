use std::collections::HashMap;

use super::super::merge::deserialize_with_merge_interp;
use crate::compose::types::ComposeFile;

fn parse_with(yaml: &str, vars: &[(&str, &str)]) -> ComposeFile {
	let vars: HashMap<String, String> = vars
		.iter()
		.map(|(k, v)| (k.to_string(), v.to_string()))
		.collect();
	deserialize_with_merge_interp(yaml, Some(&vars)).expect("the file must parse")
}

#[test]
fn bare_numbers_and_booleans_load_into_text_fields() {
	let file = parse_with(
		"services:
  web:
    image: x
    mem_limit: 512
    ports: [8080]
    expose: [3000]
    labels:
      replicas: 3
      traefik.enable: true
",
		&[],
	);
	assert_eq!(file.services["web"].expose, vec!["3000".to_string()]);
}

#[test]
fn an_interpolated_number_loads_into_a_text_field() {
	let file = parse_with(
		"services:\n  web:\n    image: x\n    command: [serve, \"${PORT}\"]\n    user: ${UID}\n",
		&[("PORT", "8080"), ("UID", "1000")],
	);
	let web = &file.services["web"];
	assert_eq!(web.user.as_deref(), Some("1000"));
}

#[test]
fn a_bare_number_in_a_string_only_key_is_still_refused() {
	let err =
		deserialize_with_merge_interp("services:\n  web:\n    image: x\n    user: 1000\n", None);
	assert!(err.is_err(), "the schema types user as a string");
}

#[test]
fn an_interpolated_value_keeps_its_text_when_a_number_would_change_it() {
	for text in ["1.10", "0x1F", "1e3", "+1"] {
		let file = parse_with(
			"services:\n  web:\n    image: x\n    environment:\n      V: \"${V}\"\n",
			&[("V", text)],
		);
		let env = format!("{:?}", file.services["web"].environment);
		assert!(env.contains(text), "{text} was rewritten: {env}");
	}
}

/// The string-only keys apply to the keys of a service, not to any key that
/// happens to carry the same name. A service called `user` still gets its
/// boolean typed, and a label called `command` is not a string-only field.
#[test]
fn a_service_named_like_a_string_key_keeps_its_fields_typed() {
	let file = parse_with(
		"services:\n  user:\n    image: x\n    privileged: \"${FLAG:-false}\"\n",
		&[],
	);
	assert_eq!(file.services["user"].privileged, Some(false));
}
