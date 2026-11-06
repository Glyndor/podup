use podup::compose::types::ServiceCondition;
use podup::{parse_str, resolve_order};

#[test]
fn no_deps() {
	let yaml =
		"services:\n  a:\n    image: alpine\n  b:\n    image: alpine\n  c:\n    image: alpine\n";
	assert_eq!(resolve_order(&parse_str(yaml).unwrap()).unwrap().len(), 3);
}

#[test]
fn linear_chain() {
	let yaml = r#"
services:
  c:
    image: alpine
    depends_on: [b]
  b:
    image: alpine
    depends_on: [a]
  a:
    image: alpine
"#;
	let order = resolve_order(&parse_str(yaml).unwrap()).unwrap();
	let pos = |s: &str| order.iter().position(|x| x == s).unwrap();
	assert!(pos("a") < pos("b"));
	assert!(pos("b") < pos("c"));
}

#[test]
fn diamond() {
	let yaml = r#"
services:
  app:
    image: alpine
    depends_on: [api, worker]
  api:
    image: alpine
    depends_on: [db]
  worker:
    image: alpine
    depends_on: [db]
  db:
    image: alpine
"#;
	let order = resolve_order(&parse_str(yaml).unwrap()).unwrap();
	let pos = |s: &str| order.iter().position(|x| x == s).unwrap();
	assert!(pos("db") < pos("api"));
	assert!(pos("db") < pos("worker"));
	assert!(pos("api") < pos("app"));
	assert!(pos("worker") < pos("app"));
}

#[test]
fn circular_dependency_error() {
	let yaml = r#"
services:
  a:
    image: alpine
    depends_on: [b]
  b:
    image: alpine
    depends_on: [a]
"#;
	assert!(resolve_order(&parse_str(yaml).unwrap()).is_err());
}

#[test]
fn missing_dependency_error() {
	let yaml = "services:\n  a:\n    image: alpine\n    depends_on: [nonexistent]\n";
	assert!(resolve_order(&parse_str(yaml).unwrap()).is_err());
}

#[test]
fn service_completed_successfully_parses() {
	let yaml = r#"
services:
  app:
    image: alpine
    depends_on:
      seed:
        condition: service_completed_successfully
  seed:
    image: alpine
"#;
	let file = parse_str(yaml).unwrap();
	let cond = file.services["app"].depends_on.condition_for("seed");
	assert_eq!(cond, ServiceCondition::ServiceCompletedSuccessfully);
}

#[test]
fn depends_on_with_required_false_on_an_undefined_service_is_an_error() {
	let yaml = r#"
services:
  app:
    image: alpine
    depends_on:
      missing:
        condition: service_started
        required: false
"#;
	// It used to be skipped. Docker Compose refuses it (`service "app" depends on
	// undefined service "missing"`): `required: false` is for a dependency that
	// exists and may be unavailable, and a name defined nowhere is a typo.
	let file = parse_str(yaml).unwrap();
	assert!(resolve_order(&file).is_err());
}

#[test]
fn depends_on_with_required_false_on_a_defined_service_is_ordered() {
	let yaml = r#"
services:
  app:
    image: alpine
    depends_on:
      db:
        condition: service_started
        required: false
  db:
    image: alpine
"#;
	let order = resolve_order(&parse_str(yaml).unwrap()).unwrap();
	assert_eq!(order, vec!["db", "app"]);
}

// ---------------------------------------------------------------------------
// Profile filtering relies on engine logic, but we can verify the parser
// preserves the active set properly.
// ---------------------------------------------------------------------------

#[test]
fn services_with_profiles_are_listed() {
	let yaml = r#"
services:
  always:
    image: alpine
  debug:
    image: alpine
    profiles: [debug]
  monitoring:
    image: alpine
    profiles: [monitoring, full]
"#;
	let file = parse_str(yaml).unwrap();
	assert!(file.services["always"].profiles.is_empty());
	assert_eq!(file.services["debug"].profiles, vec!["debug"]);
	assert!(file.services["monitoring"]
		.profiles
		.contains(&"full".to_string()));
}
