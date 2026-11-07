use super::super::deserialize_with_merge;
use crate::compose::types::EnvVars;
use crate::error::ComposeError;

fn service_with_merge(anchor: &str, merge: &str) -> String {
	format!("x-base: &base {anchor}\nservices:\n  web:\n    image: alpine\n    <<: {merge}\n")
}

#[test]
fn a_merge_of_something_that_is_not_a_mapping_is_refused() {
	// Docker Compose stops on each of these with "map merge requires map or
	// sequence of maps as the value"; dropping the key would hide a section
	// the author meant to inherit.
	let cases = [
		service_with_merge("scalar", "*base"),
		service_with_merge("[a, b]", "*base"),
		service_with_merge("{a: b}", "[*base, 5]"),
		service_with_merge("[1]", "[*base]"),
		"services:\n  web:\n    image: alpine\n    <<: null\n".to_string(),
	];
	for yaml in cases {
		let error = deserialize_with_merge(&yaml).expect_err(&yaml);
		assert!(matches!(error, ComposeError::Parse(_)), "{yaml}: {error}");
	}
}

#[test]
fn a_merge_of_mappings_still_fills_in_what_the_service_left_out() {
	let single = service_with_merge("{environment: {A: '1'}}", "*base");
	let list = service_with_merge("{environment: {A: '1'}}", "[*base]");
	let empty = "services:\n  web:\n    image: alpine\n    <<: []\n";
	for yaml in [single, list] {
		let file = deserialize_with_merge(&yaml).unwrap();
		assert!(
			matches!(file.services["web"].environment, EnvVars::Map(_)),
			"{yaml}"
		);
	}
	assert!(deserialize_with_merge(empty).is_ok());
}
