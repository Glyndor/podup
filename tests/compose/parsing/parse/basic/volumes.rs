//! `volumes`, `volumes_from` and `tmpfs`, in their short, map and list shapes.
//!
//! Split out of `tests/compose/parsing/parse/basic/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

#[test]
fn volumes_short() {
	let yaml = r#"
services:
  db:
    image: postgres:17
    volumes:
      - ./data:/var/lib/postgresql/data
      - pgdata:/var/lib/postgresql/data2
volumes:
  pgdata:
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["db"].volumes.len(), 2);
	assert!(file.volumes.contains_key("pgdata"));
}

#[test]
fn volumes_from_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    volumes_from:
      - service:ro
      - container:mycontainer
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].volumes_from.len(), 2);
}

#[test]
fn tmpfs_as_string() {
	let yaml = "services:\n  app:\n    image: alpine\n    tmpfs: /run\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].tmpfs.to_list(),
		vec!["/run"]
	);
}

#[test]
fn tmpfs_as_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    tmpfs:
      - /run
      - /tmp
"#;
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.tmpfs
			.to_list()
			.len(),
		2
	);
}
