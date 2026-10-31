//! New: extends, configs, annotations, scale
//!
//! Split out of `tests/compose/parsing/parse/schema/fields/extends_build/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// New: extends, configs, annotations, scale, complex volumes, build extras,
//        network maps, healthcheck disable, podman network modes
// ---------------------------------------------------------------------------

#[test]
fn extends_short_form() {
	let yaml = r#"
services:
  base:
    image: alpine
    environment:
      LOG: info
  app:
    extends: base
"#;
	let file = parse_str(yaml).unwrap();
	let app = &file.services["app"];
	assert_eq!(app.image.as_deref(), Some("alpine"));
	let env = app.environment.to_map();
	assert_eq!(
		env.get("LOG").and_then(|v| v.clone()).as_deref(),
		Some("info")
	);
}

#[test]
fn extends_long_form_no_file() {
	let yaml = r#"
services:
  base:
    image: alpine
  app:
    extends:
      service: base
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].image.as_deref(), Some("alpine"));
}

#[test]
fn extends_with_file_field_parses() {
	// Just verify that extends with file is parsed (resolution needs a file on disk).
	let yaml = r#"
services:
  app:
    image: alpine
    extends:
      service: base
      file: ./common.yml
"#;
	// parse_str does not resolve external files; the field should still parse,
	// but extends must remain unresolved (we expect an error from parse_str).
	let res = parse_str(yaml);
	assert!(
		res.is_err(),
		"parse_str should reject extends.file references"
	);
}

#[test]
fn configs_top_level() {
	let yaml = r#"
configs:
  app_cfg:
    file: ./app.conf
  inline_cfg:
    content: "hello world"
services:
  app:
    image: alpine
    configs:
      - app_cfg
      - source: inline_cfg
        target: /etc/inline.conf
        mode: 420
"#;
	let file = parse_str(yaml).unwrap();
	assert!(file.configs.contains_key("app_cfg"));
	assert_eq!(file.configs["app_cfg"].file.as_deref(), Some("./app.conf"));
	assert_eq!(
		file.configs["inline_cfg"].content.as_deref(),
		Some("hello world")
	);
	assert_eq!(file.services["app"].configs.len(), 2);
	assert_eq!(file.services["app"].configs[0].source(), "app_cfg");
	assert_eq!(
		file.services["app"].configs[1].target(),
		Some("/etc/inline.conf")
	);
}

#[test]
fn annotations_as_list_and_map() {
	let yaml_list = r#"
services:
  a:
    image: alpine
    annotations:
      - "io.k8s.example=foo"
"#;
	let yaml_map = r#"
services:
  a:
    image: alpine
    annotations:
      io.k8s.example: foo
"#;
	for yaml in &[yaml_list, yaml_map] {
		let m = parse_str(yaml).unwrap().services["a"].annotations.to_map();
		assert_eq!(m.get("io.k8s.example").map(|s| s.as_str()), Some("foo"));
	}
}

#[test]
fn scale_field() {
	let yaml = "services:\n  app:\n    image: alpine\n    scale: 3\n";
	assert_eq!(parse_str(yaml).unwrap().services["app"].scale, Some(3));
}
