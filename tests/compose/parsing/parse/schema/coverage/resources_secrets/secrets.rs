//! The top-level `secrets:` section and the long-form service secret
//! reference.
//!
//! Split out of `tests/compose/parsing/parse/schema/coverage/resources_secrets/mod.rs`; the imports
//! are inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// secrets top-level: content and environment
// ---------------------------------------------------------------------------

#[test]
fn secret_content_inline() {
	let yaml = r#"
secrets:
  db_password:
    content: "s3cr3t_password"
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.secrets["db_password"].content.as_deref(),
		Some("s3cr3t_password")
	);
}

#[test]
fn secret_from_environment() {
	let yaml = r#"
secrets:
  api_key:
    environment: MY_API_KEY
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.secrets["api_key"].environment.as_deref(),
		Some("MY_API_KEY")
	);
}

#[test]
fn secret_with_driver_and_labels() {
	let yaml = r#"
secrets:
  db_pass:
    driver: vault
    driver_opts:
      path: secret/db
    labels:
      team: backend
"#;
	let s = &parse_str(yaml).unwrap().secrets["db_pass"];
	assert_eq!(s.driver.as_deref(), Some("vault"));
	assert_eq!(
		s.driver_opts.get("path").map(|s| s.as_str()),
		Some("secret/db")
	);
	assert!(!s.labels.to_map().is_empty());
}

#[test]
fn secret_long_form_uid_gid_mode() {
	let yaml = r#"
secrets:
  app_cert:
    file: ./cert.pem
services:
  app:
    image: alpine
    secrets:
      - source: app_cert
        target: /run/secrets/cert.pem
        uid: "1000"
        gid: "1000"
        mode: 256
"#;
	let file = parse_str(yaml).unwrap();
	let sref = &file.services["app"].secrets[0];
	assert_eq!(sref.source(), "app_cert");
	assert_eq!(sref.target(), Some("/run/secrets/cert.pem"));
	match sref {
		ServiceSecretRef::Long { uid, gid, mode, .. } => {
			assert_eq!(uid.as_deref(), Some("1000"));
			assert_eq!(gid.as_deref(), Some("1000"));
			// The compose spec defines mode as octal: 256 == 0o256 == 174.
			assert_eq!(*mode, Some(0o256));
		}
		_ => panic!("expected long-form secret ref"),
	}
}
