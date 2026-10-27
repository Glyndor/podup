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
    user: 1000
    container_name: 123
    mem_limit: 512
    command: [serve, 8080]
    ports: [8080]
    expose: [3000]
    labels:
      replicas: 3
      traefik.enable: true
",
		&[],
	);
	let web = &file.services["web"];
	assert_eq!(web.user.as_deref(), Some("1000"));
	assert_eq!(web.container_name.as_deref(), Some("123"));
	assert_eq!(web.expose, vec!["3000".to_string()]);
}

#[test]
fn an_interpolated_number_loads_into_a_text_field() {
	let file = parse_with(
		"services:\n  web:\n    image: x\n    command: [serve, \"${PORT}\"]\n    user: ${UID}\n",
		&[("PORT", "8080"), ("UID", "1000")],
	);
	assert_eq!(file.services["web"].user.as_deref(), Some("1000"));
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
