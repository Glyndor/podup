//! Service-name diagnostics through the built binary, without a Podman daemon.

use std::process::{Command, Output};

fn config_hash(name: &str, project: Option<&str>) -> Output {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("compose.yml"),
		r#"
name: logsrepro
services:
  web: {image: nginx:alpine}
  named: {image: nginx:alpine, container_name: mi-contenedor}
"#,
	)
	.unwrap();
	let mut cmd = Command::new(env!("CARGO_BIN_EXE_podup"));
	cmd.current_dir(dir.path())
		.env_remove("COMPOSE_PROJECT_NAME")
		.args(["--ansi", "never", "-f", "compose.yml", "--socket"])
		.arg(dir.path().join("absent.sock"));
	if let Some(project) = project {
		cmd.args(["-p", project]);
	}
	cmd.args(["config", "--hash", name]).output().unwrap()
}

#[test]
fn container_name_error_is_followed_by_hint() {
	let out = config_hash("mi-contenedor", None);
	assert_eq!(out.status.code(), Some(1));
	assert_eq!(
		String::from_utf8(out.stderr).unwrap(),
		"podup: error: service 'mi-contenedor' not found\n\
		 hint: 'mi-contenedor' is the container of service 'named'; pass the service name instead\n"
	);
}

#[test]
fn replica_hint_uses_the_resolved_project() {
	let out = config_hash("my-app-web-12", Some("my-app"));
	assert_eq!(out.status.code(), Some(1));
	assert_eq!(
		String::from_utf8(out.stderr).unwrap(),
		"podup: error: service 'my-app-web-12' not found\n\
		 hint: 'my-app-web-12' is the container of service 'web'; pass the service name instead\n"
	);
}

#[test]
fn unmatched_error_output_is_unchanged() {
	for (name, project, expected) in [
		("nope", None, "podup: error: service 'nope' not found\n"),
		(
			"logsrepro-web-1",
			Some("other"),
			"podup: error: service 'logsrepro-web-1' not found\n",
		),
	] {
		let out = config_hash(name, project);
		assert_eq!(out.status.code(), Some(1));
		assert_eq!(String::from_utf8(out.stderr).unwrap(), expected);
	}
}
