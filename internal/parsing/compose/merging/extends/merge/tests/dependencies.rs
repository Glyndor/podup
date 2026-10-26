use super::*;

#[test]
fn empty_override_keeps_base_depends_on() {
	let yaml = r#"
services:
  db:
    image: postgres
  base:
    image: alpine
    depends_on:
      - db
  app:
    extends: base
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.services["app"].depends_on.service_names(),
		vec!["db".to_string()]
	);
}

#[test]
fn extends_unions_depends_on() {
	let yaml = r#"
services:
  db:
    image: postgres
  cache:
    image: redis
  base:
    image: alpine
    depends_on:
      - db
  app:
    extends: base
    depends_on:
      - cache
"#;
	let file = parse_str(yaml).unwrap();
	// compose-go unions the base and extending depends_on rather than letting
	// the override replace the base wholesale.
	let mut names = file.services["app"].depends_on.service_names();
	names.sort();
	assert_eq!(names, vec!["cache".to_string(), "db".to_string()]);
}

#[test]
fn extends_depends_on_override_wins_on_conflict() {
	let yaml = r#"
services:
  db:
    image: postgres
  base:
    image: alpine
    depends_on:
      db:
        condition: service_started
  app:
    extends: base
    depends_on:
      db:
        condition: service_healthy
"#;
	let file = parse_str(yaml).unwrap();
	// On an overlapping key the extending service's condition wins.
	assert_eq!(
		file.services["app"].depends_on.condition_for("db"),
		crate::compose::types::ServiceCondition::ServiceHealthy
	);
}

#[test]
fn depends_on_unions_when_base_has_none() {
	// The base declares no depends_on; the extending service's dependencies are
	// carried through unchanged (merge_depends_on base-empty branch).
	let yaml = r#"
services:
  base:
    image: alpine
  db:
    image: postgres
  app:
    extends: base
    depends_on:
      - db
"#;
	let file = parse_str(yaml).unwrap();
	assert!(file.services["app"]
		.depends_on
		.service_names()
		.contains(&"db".to_string()));
}
