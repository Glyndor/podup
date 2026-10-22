//! The top-level service fields that take more than one shape or reach the
//! device layer: `label_file` in both shapes, `attach`, `dns_search` as a list
//! and `device_cgroup_rules`.
//!
//! Split out of `tests/parse/coverage/network_volume/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// label_file, attach, dns_search and device_cgroup_rules
// ---------------------------------------------------------------------------

#[test]
fn label_file_single() {
	let yaml = "services:\n  app:\n    image: alpine\n    label_file: ./labels.properties\n";
	let list = parse_str(yaml).unwrap().services["app"]
		.label_file
		.to_list();
	assert_eq!(list, vec!["./labels.properties"]);
}

#[test]
fn label_file_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    label_file:
      - ./labels.properties
      - ./extra.labels
"#;
	let list = parse_str(yaml).unwrap().services["app"]
		.label_file
		.to_list();
	assert_eq!(list.len(), 2);
}

#[test]
fn attach_field() {
	let yaml = "services:\n  app:\n    image: alpine\n    attach: false\n";
	assert_eq!(parse_str(yaml).unwrap().services["app"].attach, Some(false));
}

// ---------------------------------------------------------------------------
// service.dns_search (coverage of StringOrList as list form)
// ---------------------------------------------------------------------------

#[test]
fn dns_search_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    dns_search:
      - example.com
      - internal.local
"#;
	let list = parse_str(yaml).unwrap().services["app"]
		.dns_search
		.to_list();
	assert!(list.contains(&"example.com".to_string()));
}

// ---------------------------------------------------------------------------
// Devices: cgroup rules
// ---------------------------------------------------------------------------

#[test]
fn device_cgroup_rules() {
	let yaml = r#"
services:
  app:
    image: alpine
    device_cgroup_rules:
      - "c 1:3 mr"
      - "b 7:* rmw"
"#;
	let rules = &parse_str(yaml).unwrap().services["app"].device_cgroup_rules;
	assert_eq!(rules.len(), 2);
	assert!(rules[0].contains("c 1:3"));
}
