//! `healthcheck`, `depends_on`, `hostname`, `domainname`, `mac_address` and
//! `read_only`.
//!
//! Split out of `tests/compose/parsing/parse/schema/basic/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

#[test]
fn healthcheck() {
	let yaml = r#"
services:
  db:
    image: postgres:17
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 5s
      timeout: 3s
      retries: 10
"#;
	let hc = parse_str(yaml).unwrap().services["db"]
		.healthcheck
		.as_ref()
		.unwrap()
		.retries;
	assert_eq!(hc, Some(10));
}

#[test]
fn depends_on_list() {
	let yaml = r#"
services:
  app:
    image: node:20
    depends_on: [db, redis]
  db:
    image: postgres:17
  redis:
    image: redis:8
"#;
	let deps = parse_str(yaml).unwrap().services["app"]
		.depends_on
		.service_names();
	assert!(deps.contains(&"db".to_string()));
	assert!(deps.contains(&"redis".to_string()));
}

#[test]
fn depends_on_map_with_condition() {
	let yaml = r#"
services:
  app:
    image: node:20
    depends_on:
      db:
        condition: service_healthy
  db:
    image: postgres:17
"#;
	let file = parse_str(yaml).unwrap();
	let condition = file.services["app"].depends_on.condition_for("db");
	assert!(matches!(condition, ServiceCondition::ServiceHealthy));
}

#[test]
fn hostname_and_domainname() {
	let yaml =
		"services:\n  app:\n    image: alpine\n    hostname: web1\n    domainname: example.com\n";
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].hostname.as_deref(), Some("web1"));
	assert_eq!(
		file.services["app"].domainname.as_deref(),
		Some("example.com")
	);
}

#[test]
fn mac_address() {
	let yaml = "services:\n  app:\n    image: alpine\n    mac_address: 02:42:ac:11:00:01\n";
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.services["app"].mac_address.as_deref(),
		Some("02:42:ac:11:00:01")
	);
}

#[test]
fn read_only_root() {
	let yaml = "services:\n  app:\n    image: alpine\n    read_only: true\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].read_only,
		Some(true)
	);
}
