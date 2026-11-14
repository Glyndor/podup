use super::*;
use std::collections::HashMap;

fn tagged() -> LogConfig {
	LogConfig {
		driver: None,
		size: None,
		options: HashMap::from([("tag".to_string(), "x".to_string())]),
	}
}

#[test]
fn a_host_default_of_k8s_file_or_json_file_drops_the_tag() {
	for driver in ["k8s-file", "json-file"] {
		let mut log = tagged();
		assert!(
			drop_tag_for_host_driver("web", driver, &mut log),
			"{driver}"
		);
		assert!(log.options.is_empty(), "{driver}");
	}
}

#[test]
fn a_host_default_that_takes_a_tag_keeps_it() {
	for driver in ["journald", "none", ""] {
		let mut log = tagged();
		assert!(
			!drop_tag_for_host_driver("web", driver, &mut log),
			"{driver}"
		);
		assert_eq!(log.options["tag"], "x", "{driver}");
	}
}
