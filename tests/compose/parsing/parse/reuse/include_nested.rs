//! `include:` in a file that is itself included.
//!
//! Split out of `tests/compose/parsing/parse/reuse/include.rs`.

use super::include::parse_one;

#[test]
fn a_nested_include_is_merged() {
	// The compose specification applies `include` recursively: a file that is
	// included brings its own includes with it.
	let dir = tempfile::tempdir().unwrap();

	let innermost = dir.path().join("innermost.yml");
	std::fs::write(&innermost, "services:\n  deep:\n    image: alpine\n").unwrap();

	let middle = dir.path().join("middle.yml");
	std::fs::write(
		&middle,
		"include:\n  - ./innermost.yml\n\nservices:\n  mid:\n    image: alpine\n",
	)
	.unwrap();

	let main = dir.path().join("docker-compose.yml");
	std::fs::write(
		&main,
		"include:\n  - ./middle.yml\n\nservices:\n  app:\n    image: nginx\n",
	)
	.unwrap();

	let file = parse_one(&main).unwrap();
	assert!(file.services.contains_key("app"));
	assert!(file.services.contains_key("mid"));
	assert!(
		file.services.contains_key("deep"),
		"the service of the nested include is missing: {:?}",
		file.services.keys().collect::<Vec<_>>()
	);
}

#[test]
fn a_circular_include_is_refused() {
	let dir = tempfile::tempdir().unwrap();

	let first = dir.path().join("first.yml");
	let second = dir.path().join("second.yml");
	std::fs::write(
		&first,
		"include:\n  - ./second.yml\n\nservices:\n  one:\n    image: alpine\n",
	)
	.unwrap();
	std::fs::write(
		&second,
		"include:\n  - ./first.yml\n\nservices:\n  two:\n    image: alpine\n",
	)
	.unwrap();

	let main = dir.path().join("docker-compose.yml");
	std::fs::write(
		&main,
		"include:\n  - ./first.yml\n\nservices:\n  app:\n    image: nginx\n",
	)
	.unwrap();

	let err = parse_one(&main).expect_err("a circular include must be refused");
	assert!(
		err.to_string().to_lowercase().contains("circular"),
		"got: {err}"
	);
}

#[test]
fn an_extends_inside_an_included_file_resolves_against_its_own_directory() {
	// The base file an included service extends sits next to the included
	// file, not next to the top-level compose file.
	let dir = tempfile::tempdir().unwrap();
	let sub = dir.path().join("sub");
	std::fs::create_dir_all(&sub).unwrap();

	std::fs::write(
		sub.join("base.yml"),
		"services:\n  base:\n    image: alpine\n    environment:\n      FROM: base\n",
	)
	.unwrap();

	std::fs::write(
		sub.join("services.yml"),
		r#"
services:
  app:
    extends:
      file: ./base.yml
      service: base
    environment:
      OWN: app
"#,
	)
	.unwrap();

	let main = dir.path().join("docker-compose.yml");
	std::fs::write(
		&main,
		"include:\n  - ./sub/services.yml\n\nservices:\n  web:\n    image: nginx\n",
	)
	.unwrap();

	let file = parse_one(&main).unwrap();
	let env = file.services["app"].environment.to_map();
	assert_eq!(
		env.get("FROM").and_then(|v| v.clone()).as_deref(),
		Some("base"),
		"the base service of the included file was not found: {env:?}"
	);
	assert_eq!(
		env.get("OWN").and_then(|v| v.clone()).as_deref(),
		Some("app")
	);
}
