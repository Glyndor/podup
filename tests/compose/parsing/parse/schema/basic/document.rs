//! The document itself: a minimal service, an empty map, malformed YAML.
//!
//! Split out of `tests/compose/parsing/parse/schema/basic/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

#[test]
fn minimal() {
	let file = parse_str("services:\n  web:\n    image: nginx:alpine").unwrap();
	assert!(file.services.contains_key("web"));
	assert_eq!(file.services["web"].image.as_deref(), Some("nginx:alpine"));
}

#[test]
fn empty_services() {
	assert!(parse_str("services: {}").unwrap().services.is_empty());
}

#[test]
fn invalid_yaml() {
	assert!(parse_str("services: [invalid: yaml: here").is_err());
}
