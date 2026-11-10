//! Two things docker compose does at startup that podup left out: it says when
//! more than one supported compose file name is present, and it reads an empty
//! project name (`-p ""`, `COMPOSE_PROJECT_NAME=`) as not set.

use super::*;

fn run_in(dir: &std::path::Path, args: &[&str], project_env: Option<&str>) -> Output {
	let mut cmd = Command::new(bin());
	cmd.current_dir(dir)
		.args(args)
		.env_remove("COMPOSE_FILE")
		.env_remove("COMPOSE_PROJECT_NAME")
		.env_remove("NO_COLOR")
		.env("PODMAN_SOCKET", "/nonexistent/none.sock");
	if let Some(value) = project_env {
		cmd.env("COMPOSE_PROJECT_NAME", value);
	}
	cmd.output().unwrap()
}

#[test]
fn several_supported_names_are_warned_about_and_the_first_is_used() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"services:\n  first:\n    image: x\n",
	)
	.unwrap();
	fs::write(
		dir.path().join("docker-compose.yml"),
		"services:\n  second:\n    image: x\n",
	)
	.unwrap();
	let out = run_in(dir.path(), &["config", "--services"], None);
	assert!(out.status.success(), "{out:?}");
	assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "first");
	let err = String::from_utf8_lossy(&out.stderr);
	assert!(err.contains("several compose files"), "{err}");
	assert!(
		err.contains("compose.yaml") && err.contains("docker-compose.yml"),
		"{err}"
	);
}

#[test]
fn one_file_is_not_warned_about() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"services:\n  only:\n    image: x\n",
	)
	.unwrap();
	let out = run_in(dir.path(), &["config", "--services"], None);
	assert!(
		!String::from_utf8_lossy(&out.stderr).contains("several"),
		"{out:?}"
	);
}

#[test]
fn an_empty_project_name_is_not_set() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"name: fromfile\nservices:\n  web:\n    image: x\n",
	)
	.unwrap();
	for out in [
		run_in(dir.path(), &["-p", "", "config"], None),
		run_in(dir.path(), &["config"], Some("")),
	] {
		assert!(
			out.status.success(),
			"{}",
			String::from_utf8_lossy(&out.stderr)
		);
		assert!(
			String::from_utf8_lossy(&out.stdout).contains("name: fromfile"),
			"{out:?}"
		);
	}
}
