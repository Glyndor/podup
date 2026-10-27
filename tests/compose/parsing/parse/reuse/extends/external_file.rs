//! Resolving a base service out of another file, and anchoring its relative
//! paths to that file's directory.
//!
//! Split out of `tests/compose/parsing/parse/reuse/extends/mod.rs`; the imports are inherited via
//! `use super::*`.

use std::io::Write;

use super::*;

#[test]
fn extends_with_external_file() {
	let dir = tempfile::tempdir().unwrap();
	let common_path = dir.path().join("common.yml");
	let mut f = std::fs::File::create(&common_path).unwrap();
	writeln!(
		f,
		r#"
services:
  base:
    image: alpine
    environment:
      FROM_BASE: yes
"#
	)
	.unwrap();

	let main_path = dir.path().join("docker-compose.yml");
	let mut m = std::fs::File::create(&main_path).unwrap();
	writeln!(
		m,
		r#"
services:
  app:
    extends:
      service: base
      file: ./common.yml
    environment:
      FROM_APP: yes
"#
	)
	.unwrap();

	let file = parse_files_with_env_files(&[main_path], &[]).unwrap();
	assert_eq!(file.services["app"].image.as_deref(), Some("alpine"));
	let env = file.services["app"].environment.to_map();
	assert!(env.contains_key("FROM_BASE"));
	assert!(env.contains_key("FROM_APP"));
}

#[test]
fn extends_external_file_parent_traversal_is_allowed() {
	// The compose file is trusted input: `extends.file` with `../` must resolve,
	// matching docker-compose and podman-compose (monorepo shared-base pattern).
	let dir = tempfile::tempdir().unwrap();

	let common_path = dir.path().join("common.yml");
	let mut f = std::fs::File::create(&common_path).unwrap();
	writeln!(
		f,
		r#"
services:
  base:
    image: alpine
"#
	)
	.unwrap();

	let sub = dir.path().join("stack");
	std::fs::create_dir(&sub).unwrap();
	let main_path = sub.join("docker-compose.yml");
	let mut m = std::fs::File::create(&main_path).unwrap();
	writeln!(
		m,
		r#"
services:
  app:
    extends:
      service: base
      file: ../common.yml
"#
	)
	.unwrap();

	let file = parse_files_with_env_files(&[main_path], &[]).unwrap();
	assert_eq!(file.services["app"].image.as_deref(), Some("alpine"));
}

#[test]
fn extends_external_file_anchors_relative_paths() {
	let dir = tempfile::tempdir().unwrap();
	let sub = dir.path().join("svc");
	std::fs::create_dir(&sub).unwrap();

	let common_path = sub.join("common.yml");
	let mut f = std::fs::File::create(&common_path).unwrap();
	writeln!(
		f,
		r#"
services:
  base:
    image: alpine
    env_file:
      - ./base.env
    volumes:
      - ./data:/data
"#
	)
	.unwrap();

	let main_path = dir.path().join("docker-compose.yml");
	let mut m = std::fs::File::create(&main_path).unwrap();
	writeln!(
		m,
		r#"
services:
  app:
    extends:
      service: base
      file: ./svc/common.yml
"#
	)
	.unwrap();

	let file = parse_files_with_env_files(&[main_path], &[]).unwrap();
	let app = &file.services["app"];

	// The base service's relative env_file/volume paths must be anchored to the
	// external file's directory, not the top-level project directory.
	let entries = app.env_file.to_entries();
	let env_path = std::path::Path::new(entries[0].path());
	assert!(
		env_path.is_absolute(),
		"env_file not anchored: {env_path:?}"
	);
	assert!(
		env_path.ends_with("svc/base.env"),
		"wrong dir: {env_path:?}"
	);

	let vol = match &app.volumes[0] {
		podup::compose::types::VolumeMount::Short(s) => s.clone(),
		other => panic!("expected short volume, got {other:?}"),
	};
	assert!(
		vol.contains("svc") && vol.ends_with(":/data"),
		"volume bind not anchored: {vol}"
	);
}
