//! The top-level `secrets:` section, and the service keys that reach the host:
//! `security_opt`, `devices`, `group_add`, `userns_mode`, `shm_size` and
//! `cgroup_parent`.
//!
//! Split out of `tests/compose/parsing/parse/basic/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

#[test]
fn secrets_top_level() {
	let yaml = r#"
secrets:
  db_password:
    file: ./secrets/db_password.txt
  jwt_secret:
    external: true
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.secrets["db_password"].file.as_deref(),
		Some("./secrets/db_password.txt")
	);
	assert_eq!(file.secrets["jwt_secret"].external, Some(true));
}

#[test]
fn security_opt_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    security_opt:
      - "no-new-privileges:true"
      - "label=disable"
"#;
	let so = &parse_str(yaml).unwrap().services["app"].security_opt;
	assert_eq!(so.len(), 2);
	assert!(so.iter().any(|s| s.contains("no-new-privileges")));
}

#[test]
fn devices_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    devices:
      - "/dev/sda:/dev/xvda:rwm"
      - "/dev/null:/dev/null"
"#;
	assert_eq!(parse_str(yaml).unwrap().services["app"].devices.len(), 2);
}

#[test]
fn group_add_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    group_add: [audio, video, "1000"]
"#;
	let g = &parse_str(yaml).unwrap().services["app"].group_add;
	assert_eq!(g.len(), 3);
	assert!(g.contains(&"audio".to_string()));
}

#[test]
fn userns_mode() {
	let yaml =
		"services:\n  app:\n    image: alpine\n    userns_mode: \"keep-id:uid=1000,gid=1000\"\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.userns_mode
			.as_deref(),
		Some("keep-id:uid=1000,gid=1000")
	);
}

#[test]
fn shm_size() {
	let yaml = "services:\n  app:\n    image: alpine\n    shm_size: 128m\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].shm_size.as_deref(),
		Some("128m")
	);
}

#[test]
fn cgroup_parent_field() {
	let yaml = "services:\n  app:\n    image: alpine\n    cgroup_parent: my-cgroup\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.cgroup_parent
			.as_deref(),
		Some("my-cgroup")
	);
}
