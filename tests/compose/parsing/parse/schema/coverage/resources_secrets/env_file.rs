//! `env_file` in its long form, with `required`, `format` and a mixed list.
//!
//! Split out of `tests/compose/parsing/parse/schema/coverage/resources_secrets/mod.rs`; the imports
//! are inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// env_file long-form
// ---------------------------------------------------------------------------

#[test]
fn env_file_long_form_with_required() {
	let yaml = r#"
services:
  app:
    image: alpine
    env_file:
      - path: .env.prod
        required: false
      - path: .env.local
        required: true
"#;
	let entries = parse_str(yaml).unwrap().services["app"]
		.env_file
		.to_entries();
	assert_eq!(entries.len(), 2);
	assert_eq!(entries[0].path(), ".env.prod");
	assert!(!entries[0].required());
	assert_eq!(entries[1].path(), ".env.local");
	assert!(entries[1].required());
}

#[test]
fn env_file_long_form_with_format() {
	let yaml = r#"
services:
  app:
    image: alpine
    env_file:
      - path: .env
        format: dotenv
"#;
	let entries = parse_str(yaml).unwrap().services["app"]
		.env_file
		.to_entries();
	assert_eq!(entries.len(), 1);
	assert_eq!(entries[0].path(), ".env");
}

#[test]
fn env_file_mixed_short_and_long() {
	let yaml = r#"
services:
  app:
    image: alpine
    env_file:
      - .env
      - path: .env.local
        required: false
"#;
	let entries = parse_str(yaml).unwrap().services["app"]
		.env_file
		.to_entries();
	assert_eq!(entries.len(), 2);
	assert!(entries[0].required()); // short form defaults to required
	assert!(!entries[1].required());
}
