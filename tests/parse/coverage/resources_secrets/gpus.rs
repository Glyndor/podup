//! The `gpus` shorthand and the long-form device-reservation objects.
//!
//! Split out of `tests/parse/coverage/resources_secrets/mod.rs`; the imports
//! are inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// gpus shorthand
// ---------------------------------------------------------------------------

#[test]
fn gpus_all() {
	let yaml = r#"
services:
  app:
    image: alpine
    gpus: all
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	let gpus = svc.gpus.as_ref().unwrap();
	assert_eq!(gpus.to_count(), -1);
}

#[test]
fn gpus_count() {
	let yaml = r#"
services:
  app:
    image: alpine
    gpus: 2
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	let gpus = svc.gpus.as_ref().unwrap();
	assert_eq!(gpus.to_count(), 2);
}

#[test]
fn gpus_device_object_list_form_parses() {
	// The spec also allows gpus as a list of device-reservation objects; it
	// must parse rather than error on a valid compose file.
	let yaml = r#"
services:
  app:
    image: alpine
    gpus:
      - driver: nvidia
        count: all
        capabilities: [gpu]
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	assert!(svc.gpus.is_some());
}
