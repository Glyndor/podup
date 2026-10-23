//! The three rejections a same-file `extends` can earn: a cycle, a service
//! extending itself, and a target that does not exist.
//!
//! Split out of `tests/parse/extends/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

#[test]
fn extends_circular_errors() {
	let yaml = r#"
services:
  a:
    image: alpine
    extends: b
  b:
    image: alpine
    extends: a
"#;
	assert!(parse_str(yaml).is_err());
}

#[test]
fn extends_self_errors() {
	let yaml = r#"
services:
  a:
    image: alpine
    extends: a
"#;
	assert!(parse_str(yaml).is_err());
}

#[test]
fn extends_unknown_service_errors() {
	let yaml = r#"
services:
  a:
    image: alpine
    extends: missing
"#;
	assert!(parse_str(yaml).is_err());
}
