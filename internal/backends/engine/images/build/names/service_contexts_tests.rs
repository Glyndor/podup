use super::*;

fn file(yaml: &str) -> ComposeFile {
	crate::parse_str(yaml).expect("the compose file parses")
}

const PAIR: &str = "services:
  base:
    image: proj/base:1
    build: ./base
  app:
    image: proj/app:1
    build:
      context: ./app
      additional_contexts:
        basectx: service:base
        local: ./other
";

fn names(list: &[&str]) -> Vec<String> {
	list.iter().map(|n| n.to_string()).collect()
}

#[test]
fn a_service_context_becomes_the_image_that_service_builds() {
	let file = file(PAIR);
	let build = file.services["app"].build.as_ref().unwrap();
	let resolved = resolve_additional_contexts("proj", &file, "app", build).unwrap();
	assert!(resolved.contains(&(
		"basectx".to_string(),
		"docker-image://proj/base:1".to_string()
	)));
	assert!(resolved.contains(&("local".to_string(), "./other".to_string())));
}

#[test]
fn a_service_without_an_image_uses_the_default_tag() {
	let file = file(
		"services:
  base:
    build: ./base
  app:
    build:
      context: ./app
      additional_contexts:
        basectx: service:base
",
	);
	let build = file.services["app"].build.as_ref().unwrap();
	let resolved = resolve_additional_contexts("proj", &file, "app", build).unwrap();
	assert_eq!(
		resolved,
		vec![(
			"basectx".to_string(),
			"docker-image://proj-base:latest".to_string()
		)]
	);
}

#[test]
fn a_service_that_is_not_defined_is_an_error() {
	let file = file(
		"services:
  app:
    build:
      context: ./app
      additional_contexts:
        basectx: service:missing
",
	);
	let build = file.services["app"].build.as_ref().unwrap();
	let err = resolve_additional_contexts("proj", &file, "app", build).unwrap_err();
	assert!(err.to_string().contains("\"missing\""), "{err}");
}

#[test]
fn the_service_itself_is_an_error() {
	let file = file(
		"services:
  app:
    build:
      context: ./app
      additional_contexts:
        basectx: service:app
",
	);
	let build = file.services["app"].build.as_ref().unwrap();
	let err = resolve_additional_contexts("proj", &file, "app", build).unwrap_err();
	assert!(err.to_string().contains("the service itself"), "{err}");
}

#[test]
fn the_referenced_service_is_built_first_even_when_not_asked_for() {
	let file = file(PAIR);
	assert_eq!(
		with_context_builds(&file, &names(&["app"])).unwrap(),
		names(&["base", "app"])
	);
	assert_eq!(
		with_context_builds(&file, &names(&["app", "base"])).unwrap(),
		names(&["base", "app"])
	);
}

#[test]
fn services_without_service_contexts_keep_their_order() {
	let file = file(PAIR);
	assert_eq!(
		with_context_builds(&file, &names(&["base"])).unwrap(),
		names(&["base"])
	);
}

#[test]
fn services_that_take_contexts_from_each_other_are_an_error() {
	let file = file(
		"services:
  a:
    build:
      context: ./a
      additional_contexts:
        x: service:b
  b:
    build:
      context: ./b
      additional_contexts:
        x: service:a
",
	);
	let err = with_context_builds(&file, &names(&["a"])).unwrap_err();
	assert!(err.to_string().contains("depend on each other"), "{err}");
}
