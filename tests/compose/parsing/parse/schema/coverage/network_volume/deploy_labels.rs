//! `deploy.labels` in its list and map shapes.
//!
//! Split out of `tests/compose/parsing/parse/schema/coverage/network_volume/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// deploy.labels
// ---------------------------------------------------------------------------

#[test]
fn deploy_labels_as_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      labels:
        - "com.example.description=API service"
        - "com.example.tier=backend"
"#;
	let file = parse_str(yaml).unwrap();
	let deploy = file.services["app"].deploy.as_ref().unwrap();
	let labels = deploy.labels.to_map();
	assert_eq!(
		labels.get("com.example.description").map(|s| s.as_str()),
		Some("API service")
	);
}

#[test]
fn deploy_labels_as_map() {
	let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      labels:
        app.version: "1.0"
"#;
	let file = parse_str(yaml).unwrap();
	let deploy = file.services["app"].deploy.as_ref().unwrap();
	let labels = deploy.labels.to_map();
	assert_eq!(labels.get("app.version").map(|s| s.as_str()), Some("1.0"));
}
