//! Healthcheck disable, explicit and through `test: ["NONE"]`.
//!
//! Split out of `tests/compose/parsing/parse/schema/fields/extends_build/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

#[test]
fn healthcheck_disable_via_test_none() {
	let yaml = r#"
services:
  app:
    image: alpine
    healthcheck:
      test: ["NONE"]
"#;
	let hc = parse_str(yaml).unwrap().services["app"]
		.healthcheck
		.as_ref()
		.unwrap()
		.clone();
	assert!(hc.is_disabled());
}

#[test]
fn healthcheck_disable_explicit() {
	let yaml = r#"
services:
  app:
    image: alpine
    healthcheck:
      disable: true
"#;
	let hc = parse_str(yaml).unwrap().services["app"]
		.healthcheck
		.as_ref()
		.unwrap()
		.clone();
	assert!(hc.is_disabled());
}

#[test]
fn healthcheck_start_interval() {
	let yaml = r#"
services:
  app:
    image: alpine
    healthcheck:
      test: ["CMD", "true"]
      start_interval: 1s
"#;
	let hc = parse_str(yaml).unwrap().services["app"]
		.healthcheck
		.as_ref()
		.unwrap()
		.clone();
	assert_eq!(hc.start_interval.as_deref(), Some("1s"));
}
