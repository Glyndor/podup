use podup::parse_file;
use std::io::Write;

#[test]
fn include_string_form_merges_services() {
	let dir = tempfile::tempdir().unwrap();

	let included = dir.path().join("services.yml");
	writeln!(
		std::fs::File::create(&included).unwrap(),
		r#"
services:
  helper:
    image: alpine
"#
	)
	.unwrap();

	let main = dir.path().join("docker-compose.yml");
	writeln!(
		std::fs::File::create(&main).unwrap(),
		r#"
include:
  - ./services.yml

services:
  app:
    image: nginx
"#
	)
	.unwrap();

	let file = parse_file(&main).unwrap();
	assert!(file.services.contains_key("app"));
	assert!(file.services.contains_key("helper"));
}

#[test]
fn include_parent_relative_path_resolves() {
	// The Compose Specification treats `../` as a canonical include path
	// (monorepos reference shared compose files one level up). It must resolve,
	// not be rejected as path traversal.
	let dir = tempfile::tempdir().unwrap();

	let shared = dir.path().join("shared.yml");
	writeln!(
		std::fs::File::create(&shared).unwrap(),
		r#"
services:
  shared_svc:
    image: alpine
"#
	)
	.unwrap();

	let sub = dir.path().join("project");
	std::fs::create_dir(&sub).unwrap();
	let main = sub.join("docker-compose.yml");
	writeln!(
		std::fs::File::create(&main).unwrap(),
		r#"
include:
  - ../shared.yml

services:
  app:
    image: nginx
"#
	)
	.unwrap();

	let file = parse_file(&main).unwrap();
	assert!(file.services.contains_key("app"));
	assert!(file.services.contains_key("shared_svc"));
}

#[test]
fn include_absolute_path_is_rejected() {
	// Absolute include paths remain rejected as intentional hardening: they are
	// not portable across checkouts and the spec does not require them.
	let dir = tempfile::tempdir().unwrap();
	let main = dir.path().join("docker-compose.yml");
	writeln!(
		std::fs::File::create(&main).unwrap(),
		r#"
include:
  - /etc/shared.yml

services:
  app:
    image: nginx
"#
	)
	.unwrap();

	assert!(parse_file(&main).is_err());
}

#[test]
fn include_long_form_parses() {
	let dir = tempfile::tempdir().unwrap();

	let inc = dir.path().join("inc.yml");
	writeln!(
		std::fs::File::create(&inc).unwrap(),
		r#"
services:
  inc_svc:
    image: alpine
"#
	)
	.unwrap();

	let main = dir.path().join("docker-compose.yml");
	writeln!(
		std::fs::File::create(&main).unwrap(),
		r#"
include:
  - path: ./inc.yml

services:
  main_svc:
    image: alpine
"#
	)
	.unwrap();

	let file = parse_file(&main).unwrap();
	assert!(file.services.contains_key("inc_svc"));
	assert!(file.services.contains_key("main_svc"));
}

#[test]
fn parent_overrides_included_service() {
	let dir = tempfile::tempdir().unwrap();

	let inc = dir.path().join("inc.yml");
	writeln!(
		std::fs::File::create(&inc).unwrap(),
		r#"
services:
  shared:
    image: alpine:included
"#
	)
	.unwrap();

	let main = dir.path().join("docker-compose.yml");
	writeln!(
		std::fs::File::create(&main).unwrap(),
		r#"
include:
  - ./inc.yml

services:
  shared:
    image: alpine:override
"#
	)
	.unwrap();

	let file = parse_file(&main).unwrap();
	// Parent file definition wins.
	assert_eq!(
		file.services["shared"].image.as_deref(),
		Some("alpine:override")
	);
}

#[test]
fn global_env_file_feeds_interpolation() {
	let dir = tempfile::tempdir().unwrap();

	let env_path = dir.path().join("prod.env");
	let mut e = std::fs::File::create(&env_path).unwrap();
	writeln!(e, "IMG=nginx:1.27").unwrap();

	let main_path = dir.path().join("docker-compose.yml");
	let mut m = std::fs::File::create(&main_path).unwrap();
	writeln!(m, "services:\n  web:\n    image: ${{IMG}}").unwrap();

	let file = podup::parse_file_with_env_files(&main_path, &["prod.env".to_string()]).unwrap();
	assert_eq!(file.services["web"].image.as_deref(), Some("nginx:1.27"));
}

#[test]
fn multiple_files_merge_with_override() {
	let dir = tempfile::tempdir().unwrap();

	let base = dir.path().join("base.yml");
	let mut b = std::fs::File::create(&base).unwrap();
	writeln!(
		b,
		"services:\n  web:\n    image: nginx:1.0\n    environment:\n      A: \"1\"\n"
	)
	.unwrap();

	let over = dir.path().join("override.yml");
	let mut o = std::fs::File::create(&over).unwrap();
	writeln!(
		o,
		"services:\n  web:\n    image: nginx:2.0\n    environment:\n      B: \"2\"\n  db:\n    image: postgres:16\n"
	)
	.unwrap();

	let file = podup::parse_files_with_env_files(&[base, over], &[]).unwrap();

	// Later file overrides the image and adds a service; environment keys merge.
	assert_eq!(file.services["web"].image.as_deref(), Some("nginx:2.0"));
	assert!(file.services.contains_key("db"));
	let env = file.services["web"].environment.to_map();
	assert!(env.contains_key("A"));
	assert!(env.contains_key("B"));
}
