//! The project name is a variable of the compose file: `${COMPOSE_PROJECT_NAME}`
//! resolves to the name the project runs under, whichever of the four sources
//! (`-p`, the `COMPOSE_PROJECT_NAME` variable, the top-level `name:`, the
//! directory) supplied it.
//!
//! The expectations were recorded by running `docker compose config` (Compose
//! v5.7.0) on the same files, arguments and environment; each row says which.

use std::fs;
use tempfile::tempdir;

/// `container_name: ${COMPOSE_PROJECT_NAME}-test` on a service named `a`.
const BODY: &str =
	"services:\n  a:\n    image: docker.io/library/busybox\n    container_name: ${COMPOSE_PROJECT_NAME}-test\n";

/// One row: the files, the extra inputs, and what `docker compose config` printed.
struct Row {
	label: &'static str,
	/// Text put before [`BODY`] in the single compose file.
	head: &'static str,
	/// `-p` value.
	flag: Option<&'static str>,
	/// `COMPOSE_PROJECT_NAME` in the process environment.
	env: Option<&'static str>,
	/// `FOO` in the process environment.
	var: Option<&'static str>,
	/// Content of the project `.env`.
	dotenv: Option<&'static str>,
	/// `Some((name, container_name))` printed by docker, `None` when it refuses.
	expect: Option<(&'static str, &'static str)>,
}

const NONE: Row = Row {
	label: "",
	head: "",
	flag: None,
	env: None,
	var: None,
	dotenv: None,
	expect: None,
};

fn rows() -> Vec<Row> {
	vec![
		Row {
			label: "name: only",
			head: "name: custom_project\n",
			expect: Some(("custom_project", "custom_project-test")),
			..NONE
		},
		Row {
			label: "name: and -p",
			head: "name: custom_project\n",
			flag: Some("cli"),
			expect: Some(("cli", "cli-test")),
			..NONE
		},
		Row {
			label: "name: and env var",
			head: "name: custom_project\n",
			env: Some("envn"),
			expect: Some(("envn", "envn-test")),
			..NONE
		},
		Row {
			label: "name:, env var and -p",
			head: "name: custom_project\n",
			env: Some("envn"),
			flag: Some("cli"),
			expect: Some(("cli", "cli-test")),
			..NONE
		},
		Row {
			label: "directory only",
			expect: Some(("mydir_x", "mydir_x-test")),
			..NONE
		},
		Row {
			label: "directory and -p",
			flag: Some("cli"),
			expect: Some(("cli", "cli-test")),
			..NONE
		},
		Row {
			label: "directory and env var",
			env: Some("envn"),
			expect: Some(("envn", "envn-test")),
			..NONE
		},
		Row {
			label: "name: with a default",
			head: "name: ${FOO:-fromdef}\n",
			expect: Some(("fromdef", "fromdef-test")),
			..NONE
		},
		Row {
			label: "name: from the environment",
			head: "name: ${FOO:-fromdef}\n",
			var: Some("bar"),
			expect: Some(("bar", "bar-test")),
			..NONE
		},
		Row {
			label: "name: from the .env",
			head: "name: ${FOO:-fromdef}\n",
			dotenv: Some("FOO=envf\n"),
			expect: Some(("envf", "envf-test")),
			..NONE
		},
		Row {
			label: "name: refers to itself, with a default",
			head: "name: ${COMPOSE_PROJECT_NAME:-dflt}\n",
			expect: Some(("dflt", "dflt-test")),
			..NONE
		},
		Row {
			label: "name: refers to itself, with -p",
			head: "name: ${COMPOSE_PROJECT_NAME:-dflt}\n",
			flag: Some("cli"),
			expect: Some(("cli", "cli-test")),
			..NONE
		},
		Row {
			label: "name: refers to itself, with env var",
			head: "name: ${COMPOSE_PROJECT_NAME}\n",
			env: Some("envn"),
			expect: Some(("envn", "envn-test")),
			..NONE
		},
		Row {
			label: "name: refers to itself, with -p and no default",
			head: "name: ${COMPOSE_PROJECT_NAME}\n",
			flag: Some("cli"),
			expect: Some(("cli", "cli-test")),
			..NONE
		},
		Row {
			label: "name: refers to itself, nothing to give it",
			head: "name: ${COMPOSE_PROJECT_NAME}\n",
			expect: None,
			..NONE
		},
	]
}

fn run(row: &Row, args_extra: &[&str]) -> std::process::Output {
	let tmp = tempdir().unwrap();
	let dir = tmp.path().join("My.Dir_x");
	fs::create_dir(&dir).unwrap();
	fs::write(dir.join("c.yaml"), format!("{}{BODY}", row.head)).unwrap();
	if let Some(dotenv) = row.dotenv {
		fs::write(dir.join(".env"), dotenv).unwrap();
	}
	let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_podup"));
	cmd.current_dir(&dir)
		.env("NO_COLOR", "1")
		.env_remove("COMPOSE_PROJECT_NAME")
		.env_remove("COMPOSE_FILE")
		.env_remove("COMPOSE_PROFILES")
		.env_remove("FOO");
	if let Some(env) = row.env {
		cmd.env("COMPOSE_PROJECT_NAME", env);
	}
	if let Some(var) = row.var {
		cmd.env("FOO", var);
	}
	cmd.args(["-f", "c.yaml"]);
	if let Some(flag) = row.flag {
		cmd.args(["-p", flag]);
	}
	cmd.arg("config").args(args_extra).output().unwrap()
}

/// Every row prints what `docker compose config` printed.
#[test]
fn config_matches_docker_for_every_source_of_the_name() {
	for row in rows() {
		let out = run(&row, &[]);
		let stdout = String::from_utf8_lossy(&out.stdout);
		let stderr = String::from_utf8_lossy(&out.stderr);
		match row.expect {
			Some((name, container)) => {
				assert!(out.status.success(), "{}: failed: {stderr}", row.label);
				let value: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
				assert_eq!(value["name"].as_str(), Some(name), "{}: name", row.label);
				assert_eq!(
					value["services"]["a"]["container_name"].as_str(),
					Some(container),
					"{}: container_name\n{stdout}",
					row.label
				);
				assert!(
					!stderr.contains("COMPOSE_PROJECT_NAME"),
					"{}: warned about the variable: {stderr}",
					row.label
				);
			}
			None => assert!(!out.status.success(), "{}: accepted: {stdout}", row.label),
		}
	}
}

/// A `.env` that sets `COMPOSE_PROJECT_NAME` still feeds the interpolation, as
/// docker does (`dotenvn-test`); the process variable and `-p` come first.
#[test]
fn a_dotenv_value_still_feeds_the_interpolation() {
	let row = Row {
		head: "name: custom_project\n",
		dotenv: Some("COMPOSE_PROJECT_NAME=dotenvn\n"),
		..NONE
	};
	let value: serde_yaml::Value = serde_yaml::from_slice(&run(&row, &[]).stdout).unwrap();
	assert_eq!(
		value["services"]["a"]["container_name"].as_str(),
		Some("dotenvn-test")
	);
	let row = Row {
		flag: Some("cli"),
		..row
	};
	let value: serde_yaml::Value = serde_yaml::from_slice(&run(&row, &[]).stdout).unwrap();
	assert_eq!(
		value["services"]["a"]["container_name"].as_str(),
		Some("cli-test")
	);
}

/// With several `-f` files the name of the last file that sets one wins, and
/// every file sees it.
#[test]
fn the_name_of_the_last_file_wins() {
	let tmp = tempdir().unwrap();
	fs::write(tmp.path().join("a.yaml"), format!("name: first\n{BODY}")).unwrap();
	fs::write(
		tmp.path().join("b.yaml"),
		"name: second\nservices:\n  b:\n    image: docker.io/library/busybox\n    container_name: ${COMPOSE_PROJECT_NAME}-b\n",
	)
	.unwrap();
	let out = std::process::Command::new(env!("CARGO_BIN_EXE_podup"))
		.current_dir(tmp.path())
		.env("NO_COLOR", "1")
		.env_remove("COMPOSE_PROJECT_NAME")
		.env_remove("COMPOSE_FILE")
		.args(["-f", "a.yaml", "-f", "b.yaml", "config"])
		.output()
		.unwrap();
	let value: serde_yaml::Value = serde_yaml::from_slice(&out.stdout).unwrap();
	assert_eq!(value["name"].as_str(), Some("second"));
	assert_eq!(
		value["services"]["a"]["container_name"].as_str(),
		Some("second-test")
	);
	assert_eq!(
		value["services"]["b"]["container_name"].as_str(),
		Some("second-b")
	);
}

/// The name reaches the places that build their own variables: an `--env-file`
/// (which replaces the `.env`) and an included file.
#[test]
fn the_name_reaches_env_files_and_included_files() {
	let tmp = tempdir().unwrap();
	let dir = tmp.path().join("My.Dir_x");
	fs::create_dir(&dir).unwrap();
	fs::write(dir.join("vars.env"), "OTHER=1\n").unwrap();
	fs::write(
		dir.join("c.yaml"),
		format!("name: custom_project\ninclude:\n  - inc.yaml\n{BODY}"),
	)
	.unwrap();
	fs::write(
		dir.join("inc.yaml"),
		"services:\n  b:\n    image: docker.io/library/busybox\n    container_name: ${COMPOSE_PROJECT_NAME}-inc\n",
	)
	.unwrap();
	let out = std::process::Command::new(env!("CARGO_BIN_EXE_podup"))
		.current_dir(&dir)
		.env("NO_COLOR", "1")
		.env_remove("COMPOSE_PROJECT_NAME")
		.env_remove("COMPOSE_FILE")
		.args([
			"--env-file",
			"vars.env",
			"-f",
			"c.yaml",
			"-p",
			"cli",
			"config",
		])
		.output()
		.unwrap();
	let value: serde_yaml::Value = serde_yaml::from_slice(&out.stdout).unwrap();
	assert_eq!(
		value["services"]["a"]["container_name"].as_str(),
		Some("cli-test")
	);
	assert_eq!(
		value["services"]["b"]["container_name"].as_str(),
		Some("cli-inc")
	);
}
