//! `environment`, written as a list and as a map.
//!
//! Split out of `tests/compose/parsing/parse/schema/basic/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

#[test]
fn env_as_list() {
	let yaml = r#"
services:
  app:
    image: node:20
    environment:
      - NODE_ENV=production
      - PORT=3000
      - SECRET
"#;
	let env = parse_str(yaml).unwrap().services["app"]
		.environment
		.to_map();
	assert_eq!(env["NODE_ENV"].as_deref(), Some("production"));
	assert_eq!(env["PORT"].as_deref(), Some("3000"));
	assert!(env.contains_key("SECRET"));
}

#[test]
fn env_as_map() {
	let yaml = r#"
services:
  app:
    image: node:20
    environment:
      NODE_ENV: production
      PORT: 3000
"#;
	let env = parse_str(yaml).unwrap().services["app"]
		.environment
		.to_map();
	assert_eq!(env["NODE_ENV"].as_deref(), Some("production"));
	assert_eq!(env["PORT"].as_deref(), Some("3000"));
}
