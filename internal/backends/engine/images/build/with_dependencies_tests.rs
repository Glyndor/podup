//! `build --with-dependencies` builds the services the named ones depend on.
//!
//! `docker compose build web` builds only `web`; with `--with-dependencies` it
//! also builds what `web` depends on, transitively. Without service names every
//! service is built anyway.

use super::build_target_names;

const FILE: &str = "\
services:
  web:
    build: ./web
    depends_on: [api]
  api:
    build: ./api
    depends_on: [db]
  db:
    build: ./db
  other:
    build: ./other
";

fn names(services: &[&str], with_dependencies: bool) -> Vec<String> {
	let file = crate::parse_str(FILE).unwrap();
	let services: Vec<String> = services.iter().map(|s| s.to_string()).collect();
	let mut out = build_target_names(&file, &services, with_dependencies).unwrap();
	out.sort();
	out
}

#[test]
fn without_the_flag_only_the_named_service_is_built() {
	assert_eq!(names(&["web"], false), vec!["web"]);
}

#[test]
fn with_the_flag_the_dependencies_are_built_transitively() {
	assert_eq!(names(&["web"], true), vec!["api", "db", "web"]);
}

#[test]
fn with_the_flag_a_service_without_dependencies_is_built_alone() {
	assert_eq!(names(&["db"], true), vec!["db"]);
}

#[test]
fn with_the_flag_and_no_names_every_service_is_built() {
	assert_eq!(names(&[], true), vec!["api", "db", "other", "web"]);
}

#[test]
fn an_unknown_service_is_still_refused_with_the_flag() {
	let file = crate::parse_str(FILE).unwrap();
	let err = build_target_names(&file, &["nope".to_string()], true).unwrap_err();
	assert!(matches!(
		err,
		crate::error::ComposeError::ServiceNotFound(_)
	));
}
