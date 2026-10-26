//! The `isolation`, `entitlements` and `sbom` build fields.
//!
//! Split out of `tests/compose/parsing/parse/coverage/network_volume/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// Build: isolation, entitlements, provenance, sbom
// ---------------------------------------------------------------------------

#[test]
fn build_isolation() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      isolation: hyperv
"#;
	match parse_str(yaml).unwrap().services["app"]
		.build
		.as_ref()
		.unwrap()
	{
		BuildConfig::Config { isolation, .. } => assert_eq!(isolation.as_deref(), Some("hyperv")),
		_ => panic!("expected long-form build"),
	}
}

#[test]
fn build_entitlements() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      entitlements:
        - network.host
        - security.insecure
"#;
	match parse_str(yaml).unwrap().services["app"]
		.build
		.as_ref()
		.unwrap()
	{
		BuildConfig::Config { entitlements, .. } => {
			assert_eq!(entitlements.len(), 2);
			assert!(entitlements.contains(&"network.host".to_string()));
		}
		_ => panic!("expected long-form build"),
	}
}

#[test]
fn build_sbom() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      sbom: true
"#;
	match parse_str(yaml).unwrap().services["app"]
		.build
		.as_ref()
		.unwrap()
	{
		BuildConfig::Config { sbom, .. } => assert_eq!(*sbom, Some(true)),
		_ => panic!("expected long-form build"),
	}
}
