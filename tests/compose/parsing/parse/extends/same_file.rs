//! Merging and overriding inside one file.
//!
//! Split out of `tests/compose/parsing/parse/extends/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

#[test]
fn extends_same_file() {
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
	assert_eq!(file.services["app"].image.as_deref(), Some("alpine"));
}

#[test]
fn extends_merges_environment() {
	let yaml = r#"
services:
  base:
    image: alpine
    environment:
      LOG: info
      KEEP: yes
  app:
    extends:
      service: base
    environment:
      LOG: debug
      EXTRA: ok
"#;
	let file = parse_str(yaml).unwrap();
	let env = file.services["app"].environment.to_map();
	// Override wins for LOG, base value preserved for KEEP, override-only EXTRA present.
	assert_eq!(
		env.get("LOG").and_then(|v| v.clone()).as_deref(),
		Some("debug")
	);
	assert_eq!(
		env.get("KEEP").and_then(|v| v.clone()).as_deref(),
		Some("yes")
	);
	assert_eq!(
		env.get("EXTRA").and_then(|v| v.clone()).as_deref(),
		Some("ok")
	);
}

#[test]
fn extends_override_image_wins() {
	let yaml = r#"
services:
  base:
    image: alpine
  app:
    extends: base
    image: nginx:alpine
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].image.as_deref(), Some("nginx:alpine"));
}

#[test]
fn extends_chains_through_multiple_levels() {
	let yaml = r#"
services:
  grand:
    image: alpine
    environment:
      A: 1
  parent:
    extends: grand
    environment:
      B: 2
  child:
    extends: parent
    environment:
      C: 3
"#;
	let file = parse_str(yaml).unwrap();
	let env = file.services["child"].environment.to_map();
	assert!(env.contains_key("A"));
	assert!(env.contains_key("B"));
	assert!(env.contains_key("C"));
	assert_eq!(file.services["child"].image.as_deref(), Some("alpine"));
}
