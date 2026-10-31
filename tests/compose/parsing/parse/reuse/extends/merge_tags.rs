//! The `!override` and `!reset` tags in an `extends:` merge.
//!
//! Split out of `tests/compose/parsing/parse/reuse/extends/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;
use podup::compose::types::PortMapping;

#[test]
fn override_replaces_the_base_ports() {
	let yaml = r#"
services:
  base:
    image: alpine
    ports:
      - "8080:80"
  app:
    extends:
      service: base
    ports: !override
      - "9090:80"
"#;
	let file = parse_str(yaml).unwrap();
	let ports = &file.services["app"].ports;
	assert_eq!(ports.len(), 1, "the base's ports must not be appended");
	assert!(
		matches!(&ports[0], PortMapping::Short(s) if s == "9090:80"),
		"got: {ports:?}"
	);
}

#[test]
fn reset_drops_the_base_dns() {
	let yaml = r#"
services:
  base:
    image: alpine
    dns:
      - 8.8.8.8
  app:
    extends:
      service: base
    dns: !reset []
"#;
	let file = parse_str(yaml).unwrap();
	assert!(
		file.services["app"].dns.is_empty(),
		"the base's dns must be dropped"
	);
}

#[test]
fn override_in_an_external_extends_replaces_the_base_ports() {
	let dir = tempfile::tempdir().unwrap();
	let base = dir.path().join("base.yml");
	std::fs::write(
		&base,
		r#"
services:
  base:
    image: alpine
    ports:
      - "8080:80"
"#,
	)
	.unwrap();
	let main = dir.path().join("main.yml");
	std::fs::write(
		&main,
		r#"
services:
  app:
    extends:
      file: base.yml
      service: base
    ports: !override
      - "9090:80"
"#,
	)
	.unwrap();
	let file = parse_files_with_env_files(&[main], &[]).unwrap();
	let ports = &file.services["app"].ports;
	assert_eq!(ports.len(), 1, "the base's ports must not be appended");
	assert!(
		matches!(&ports[0], PortMapping::Short(s) if s == "9090:80"),
		"got: {ports:?}"
	);
}
