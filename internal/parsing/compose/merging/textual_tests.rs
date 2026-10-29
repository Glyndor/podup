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

/// A service built from a merge key has its fields only once the key is
/// resolved. Typing them before that left `user: "${UID}"` read as a number.
#[test]
fn a_service_built_from_a_merge_key_keeps_its_fields_typed() {
	let file = parse_with(
		"x-base: &base\n  image: x\n  user: ${UID}\nservices:\n  web:\n    <<: *base\n",
		&[("UID", "1000")],
	);
	assert_eq!(file.services["web"].user.as_deref(), Some("1000"));
}

/// `services: !override` is a tagged mapping and must be typed like a plain one.
#[test]
fn a_tagged_services_section_keeps_its_fields_typed() {
	let file = parse_with(
		"services: !override\n  web:\n    image: x\n    user: ${UID}\n",
		&[("UID", "1000")],
	);
	assert_eq!(file.services["web"].user.as_deref(), Some("1000"));
}

/// Numbers are valid where the schema says "string or number": a group id, a
/// logging option, a driver option, a health-check argument that is only a
/// substitution. They used to stop the whole file from loading.
#[test]
fn numbers_load_as_text_in_the_remaining_text_fields() {
	let file = parse_with(
		"services:
  web:
    image: x
    group_add: [999, \"${DOCKER_GID}\"]
    healthcheck:
      test: [CMD, nc, -z, localhost, \"${PORT}\"]
    logging:
      driver: json-file
      options:
        max-file: 3
        max-size: 10
    build:
      context: .
      shm_size: 67108864
      args:
        N: 4
networks:
  net:
    driver_opts:
      mtu: 1450
volumes:
  vol:
    driver_opts:
      size: 100
",
		&[("DOCKER_GID", "998"), ("PORT", "8080")],
	);
	let web = &file.services["web"];
	assert_eq!(web.group_add, ["999", "998"]);
	let test = web.healthcheck.as_ref().unwrap().test.clone();
	assert!(format!("{test:?}").contains("8080"), "{test:?}");
}

/// A merge key inside a tagged section: the tag hides the mapping from the
/// merge-key pass, so the service lost what it inherited.
#[test]
fn a_service_built_from_a_merge_key_inside_a_tagged_section_keeps_what_it_inherits() {
	let file = parse_with(
		"x-base: &base\n  image: x\n  user: ${UID}\nservices: !override\n  web:\n    <<: *base\n",
		&[("UID", "1000")],
	);
	let web = &file.services["web"];
	assert_eq!(web.image.as_deref(), Some("x"));
	assert_eq!(web.user.as_deref(), Some("1000"));
}

#[test]
fn a_tagged_service_built_from_a_merge_key_keeps_what_it_inherits() {
	let file = parse_with(
		"x-base: &base\n  image: x\n  user: ${UID}\nservices:\n  web: !override\n    <<: *base\n",
		&[("UID", "1000")],
	);
	let web = &file.services["web"];
	assert_eq!(web.image.as_deref(), Some("x"));
	assert_eq!(web.user.as_deref(), Some("1000"));
}
