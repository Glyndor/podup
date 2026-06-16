use podup::parse_file;
use podup::parse_str;
use std::io::Write;

#[test]
fn yaml_anchor_and_alias() {
	let yaml = r#"
x-common: &common
  image: alpine
  restart: always
  environment:
    LOG_LEVEL: info

services:
  web:
    <<: *common
  api:
    <<: *common
    image: node:20
"#;
	let file = parse_str(yaml).unwrap();

	// web inherits image and environment from anchor.
	assert_eq!(file.services["web"].image.as_deref(), Some("alpine"));
	assert!(file.services["web"]
		.environment
		.to_map()
		.contains_key("LOG_LEVEL"));

	// api overrides image, keeps environment.
	assert_eq!(file.services["api"].image.as_deref(), Some("node:20"));
	assert!(file.services["api"]
		.environment
		.to_map()
		.contains_key("LOG_LEVEL"));
}

#[test]
fn yaml_anchor_passthrough_for_environment() {
	let yaml = r#"
x-env: &env
  NODE_ENV: production
  PORT: "3000"

services:
  app:
    image: node
    environment: *env
"#;
	let file = parse_str(yaml).unwrap();
	let env = file.services["app"].environment.to_map();
	assert_eq!(
		env.get("NODE_ENV").and_then(|v| v.clone()).as_deref(),
		Some("production")
	);
	assert_eq!(
		env.get("PORT").and_then(|v| v.clone()).as_deref(),
		Some("3000")
	);
}

#[test]
fn yaml_merge_key_sequence_of_anchors() {
	let yaml = r#"
x-a: &a
  restart: always
x-b: &b
  environment:
    FROM_B: "yes"

services:
  app:
    image: alpine
    <<: [*a, *b]
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.services["app"]
			.restart
			.as_ref()
			.map(|r| format!("{r:?}")),
		Some("Always".to_string())
	);
	assert!(file.services["app"]
		.environment
		.to_map()
		.contains_key("FROM_B"));
}

#[test]
fn include_missing_path_errors() {
	// `include:` resolves absolute and `../` paths (trusted input); a path that
	// does not exist still fails cleanly rather than being silently ignored.
	let dir = tempfile::tempdir().unwrap();
	let main = dir.path().join("docker-compose.yml");
	writeln!(
		std::fs::File::create(&main).unwrap(),
		"include:\n  - /nonexistent/podup-does-not-exist.yml\nservices:\n  app:\n    image: alpine"
	)
	.unwrap();
	assert!(parse_file(&main).is_err());
}

#[test]
fn extends_file_absolute_path_rejected() {
	let dir = tempfile::tempdir().unwrap();
	let main = dir.path().join("docker-compose.yml");
	writeln!(
		std::fs::File::create(&main).unwrap(),
		"services:\n  app:\n    extends:\n      service: base\n      file: /etc/shadow"
	)
	.unwrap();
	assert!(parse_file(&main).is_err());
}

#[test]
fn extends_file_parent_traversal_rejected() {
	let dir = tempfile::tempdir().unwrap();
	let main = dir.path().join("docker-compose.yml");
	writeln!(
		std::fs::File::create(&main).unwrap(),
		"services:\n  app:\n    extends:\n      service: base\n      file: ../../other.yml"
	)
	.unwrap();
	assert!(parse_file(&main).is_err());
}
