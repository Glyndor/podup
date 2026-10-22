//! The top-level `configs:` section: `content`, `environment`, and the
//! long-form service reference with its `uid`, `gid` and `mode`.
//!
//! Split out of `tests/parse/coverage/deploy_build/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// configs: content and environment
// ---------------------------------------------------------------------------

#[test]
fn config_content_multiline() {
	let yaml = r#"
configs:
  app_conf:
    content: |
      [server]
      port = 8080
"#;
	let content = parse_str(yaml).unwrap().configs["app_conf"]
		.content
		.clone()
		.unwrap();
	assert!(content.contains("port = 8080"));
}

#[test]
fn config_from_environment() {
	let yaml = r#"
configs:
  token:
    environment: AUTH_TOKEN
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.configs["token"].environment.as_deref(),
		Some("AUTH_TOKEN")
	);
}

#[test]
fn config_long_form_uid_gid_mode() {
	let yaml = r#"
configs:
  app_cfg:
    file: ./app.conf
services:
  app:
    image: alpine
    configs:
      - source: app_cfg
        target: /etc/app.conf
        uid: "500"
        gid: "500"
        mode: 292
"#;
	let file = parse_str(yaml).unwrap();
	let cref = &file.services["app"].configs[0];
	assert_eq!(cref.source(), "app_cfg");
	assert_eq!(cref.target(), Some("/etc/app.conf"));
	match cref {
		ServiceConfigRef::Long { uid, gid, mode, .. } => {
			assert_eq!(uid.as_deref(), Some("500"));
			assert_eq!(gid.as_deref(), Some("500"));
			assert_eq!(*mode, Some(292));
		}
		_ => panic!("expected long-form config ref"),
	}
}
