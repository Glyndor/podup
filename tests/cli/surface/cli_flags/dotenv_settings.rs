//! The settings the tool reads for itself (`COMPOSE_FILE`, `COMPOSE_PATH_SEPARATOR`,
//! `COMPOSE_PROJECT_NAME`, `COMPOSE_PROFILES`, `COMPOSE_ENV_FILES`) also come from
//! the project's `.env`. A flag beats the process environment, and that beats the
//! env file. Values of an env file refer to the keys before them and to the process.

use super::*;

const BASE: &str = "services:\n  base:\n    image: x\n  dbg:\n    image: x\n    profiles: [dbg]\n";
const OTHER: &str = "services:\n  other:\n    image: x\n";

fn project(env_file: &str) -> tempfile::TempDir {
	let dir = tempfile::tempdir().unwrap();
	fs::write(dir.path().join("compose.yaml"), BASE).unwrap();
	fs::write(dir.path().join("other.yaml"), OTHER).unwrap();
	fs::write(dir.path().join(".env"), env_file).unwrap();
	dir
}

fn run(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> Output {
	let mut cmd = Command::new(bin());
	cmd.current_dir(dir).args(args);
	for name in [
		"COMPOSE_FILE",
		"COMPOSE_PATH_SEPARATOR",
		"COMPOSE_PROJECT_NAME",
		"COMPOSE_PROFILES",
		"COMPOSE_ENV_FILES",
		"NO_COLOR",
	] {
		cmd.env_remove(name);
	}
	cmd.env("PODMAN_SOCKET", "/nonexistent/none.sock");
	for (k, v) in env {
		cmd.env(k, v);
	}
	cmd.output().unwrap()
}

fn stdout(out: &Output) -> String {
	assert!(out.status.success(), "{out:?}");
	String::from_utf8_lossy(&out.stdout).into_owned()
}

fn services(out: &Output) -> Vec<String> {
	let mut names: Vec<String> = stdout(out).lines().map(str::to_string).collect();
	names.sort();
	names
}

fn name_of(out: &Output) -> String {
	stdout(out).lines().next().unwrap_or_default().to_string()
}

#[test]
fn env_file_values_refer_to_earlier_keys_and_the_process() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"services:\n  s:\n    image: x\n    environment:\n      B: \"${B}\"\n      C: \"${C}\"\n      D: \"${D}\"\n      E: \"${E}\"\n",
	)
	.unwrap();
	fs::write(
		dir.path().join(".env"),
		"A=Hello\nB=${A} World\nC=${NOPE_UNSET:-dflt}\nD='${A}'\nE=$FROM_PROCESS-x\n",
	)
	.unwrap();
	let out = run(dir.path(), &["config"], &[("FROM_PROCESS", "proc")]);
	let text = stdout(&out);
	assert!(text.contains("B: Hello World"), "{text}");
	assert!(text.contains("C: dflt"), "{text}");
	assert!(text.contains("D: $${A}"), "{text}");
	assert!(text.contains("E: proc-x"), "{text}");
}

#[test]
fn compose_file_comes_from_the_env_file_and_skips_the_override() {
	// The default separator is `:` on Unix and `;` on Windows.
	let sep = if cfg!(windows) { ';' } else { ':' };
	let dir = project(&format!("COMPOSE_FILE=other.yaml{sep}compose.yaml\n"));
	fs::write(
		dir.path().join("compose.override.yaml"),
		OTHER.replace("other", "over"),
	)
	.unwrap();
	let out = run(dir.path(), &["config", "--services"], &[]);
	assert_eq!(services(&out), ["base", "other"]);
}

#[test]
fn compose_path_separator_applies_to_the_process_and_to_the_env_file() {
	let dir = project("");
	let out = run(
		dir.path(),
		&["config", "--services"],
		&[
			("COMPOSE_FILE", "other.yaml;compose.yaml"),
			("COMPOSE_PATH_SEPARATOR", ";"),
		],
	);
	assert_eq!(services(&out), ["base", "other"]);

	let dir = project("COMPOSE_PATH_SEPARATOR=;\nCOMPOSE_FILE=other.yaml;compose.yaml\n");
	let out = run(dir.path(), &["config", "--services"], &[]);
	assert_eq!(services(&out), ["base", "other"]);
}

#[test]
fn compose_file_flag_and_process_beat_the_env_file() {
	let dir = project("COMPOSE_FILE=other.yaml\n");
	let out = run(
		dir.path(),
		&["-f", "compose.yaml", "config", "--services"],
		&[],
	);
	assert_eq!(services(&out), ["base"]);
	let out = run(
		dir.path(),
		&["config", "--services"],
		&[("COMPOSE_FILE", "compose.yaml")],
	);
	assert_eq!(services(&out), ["base"]);
}

#[test]
fn project_name_comes_from_the_env_file_below_the_flag_and_the_process() {
	let dir = project("COMPOSE_PROJECT_NAME=fromenv\n");
	assert_eq!(name_of(&run(dir.path(), &["config"], &[])), "name: fromenv");
	let out = run(dir.path(), &["config"], &[("COMPOSE_PROJECT_NAME", "proc")]);
	assert_eq!(name_of(&out), "name: proc");
	let out = run(dir.path(), &["-p", "cli", "config"], &[]);
	assert_eq!(name_of(&out), "name: cli");
}

#[test]
fn profiles_come_from_the_env_file_below_the_flag_and_the_process() {
	let dir = project("COMPOSE_PROFILES=dbg\n");
	let out = run(dir.path(), &["config", "--services"], &[]);
	assert_eq!(services(&out), ["base", "dbg"]);
	let out = run(
		dir.path(),
		&["config", "--services"],
		&[("COMPOSE_PROFILES", "none")],
	);
	assert_eq!(services(&out), ["base"]);
	let out = run(
		dir.path(),
		&["--profile", "none", "config", "--services"],
		&[],
	);
	assert_eq!(services(&out), ["base"]);
}

#[test]
fn compose_env_files_acts_as_repeated_env_file_flags() {
	let dir = project("COMPOSE_PROJECT_NAME=fromdotenv\n");
	fs::write(dir.path().join("a.env"), "SUBJECT=World\nGREETING=Hello\n").unwrap();
	fs::write(dir.path().join("b.env"), "SUBJECT=Podman\n").unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"services:\n  s:\n    image: x\n    command: [\"echo\", \"${GREETING} ${SUBJECT}\"]\n",
	)
	.unwrap();
	let out = run(
		dir.path(),
		&["config"],
		&[("COMPOSE_ENV_FILES", "a.env,b.env")],
	);
	let text = stdout(&out);
	assert!(text.contains("Hello Podman"), "{text}");
	// The listed files replace `.env`, so its project name is not read.
	assert!(!text.contains("fromdotenv"), "{text}");
	// An explicit flag wins over the variable.
	let out = run(
		dir.path(),
		&["--env-file", "a.env", "config"],
		&[("COMPOSE_ENV_FILES", "a.env,b.env")],
	);
	assert!(stdout(&out).contains("Hello World"));
}

#[test]
fn a_setting_in_an_env_file_replaced_by_the_flag_is_not_read() {
	let dir = project("COMPOSE_PROFILES=dbg\n");
	fs::write(dir.path().join("x.env"), "OTHER=1\n").unwrap();
	let out = run(
		dir.path(),
		&["--env-file", "x.env", "config", "--services"],
		&[],
	);
	assert_eq!(services(&out), ["base"]);
}
