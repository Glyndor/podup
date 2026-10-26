//! Build extras: cache_from, shm_size, network and platforms.
//!
//! Split out of `tests/compose/parsing/parse/fields/extends_build/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

#[test]
fn build_cache_from() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      cache_from:
        - registry/example:latest
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	match svc.build.as_ref().unwrap() {
		BuildConfig::Config { cache_from, .. } => assert_eq!(cache_from.len(), 1),
		_ => panic!("expected long-form build"),
	}
}

#[test]
fn build_shm_size() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      shm_size: 128m
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	match svc.build.as_ref().unwrap() {
		BuildConfig::Config { shm_size, .. } => assert_eq!(shm_size.as_deref(), Some("128m")),
		_ => panic!(""),
	}
}

#[test]
fn build_network() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      network: host
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	match svc.build.as_ref().unwrap() {
		BuildConfig::Config { network, .. } => assert_eq!(network.as_deref(), Some("host")),
		_ => panic!(""),
	}
}

#[test]
fn build_platforms() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      platforms:
        - linux/amd64
        - linux/arm64
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	match svc.build.as_ref().unwrap() {
		BuildConfig::Config { platforms, .. } => assert_eq!(platforms.len(), 2),
		_ => panic!(""),
	}
}
