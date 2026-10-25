//! A job gets every pass a service gets when `config` reads a file.
//!
//! A job is a service plus triggers, so the passes that fold `env_file` into
//! the environment, anchor a path to the file that wrote it, resolve a bind
//! source, attach the default network, warn about an unknown key or a host
//! mode, and resolve `extends` across files have to reach it. Each test makes
//! the pass observable on a job; a pass that skips jobs leaves the job
//! looking like the raw file.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// A socket that cannot be connected to. Every command these tests run
/// names it, so none of them can reach a real Podman.
const NO_ENGINE: &str = "unix:///nonexistent/x.sock";

fn podup(dir: &Path, file: &str, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["--socket", NO_ENGINE])
		.current_dir(dir)
		.args(["-f", file])
		.args(args)
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup")
}

fn text(bytes: &[u8]) -> String {
	String::from_utf8_lossy(bytes).into_owned()
}

fn config_json(dir: &Path, file: &str) -> serde_json::Value {
	let out = podup(dir, file, &["config", "--format", "json"]);
	assert!(out.status.success(), "{}", text(&out.stderr));
	serde_json::from_str(&text(&out.stdout)).expect("config json")
}

#[test]
fn env_file_is_folded_into_the_environment_of_a_job() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(dir.path().join("job.env"), "FROM_FILE=yes-from-file\n").unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"jobs:\n  j:\n    image: x\n    env_file: job.env\n    triggers: {manual: true}\n",
	)
	.unwrap();
	let json = config_json(dir.path(), "compose.yaml");
	assert_eq!(
		json["jobs"]["j"]["environment"]["FROM_FILE"],
		"yes-from-file"
	);
}

#[test]
fn a_relative_bind_source_of_a_job_is_made_absolute() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"jobs:\n  j:\n    image: x\n    volumes: ['./data:/data']\n    triggers: {manual: true}\n",
	)
	.unwrap();
	let json = config_json(dir.path(), "compose.yaml");
	let volume = json["jobs"]["j"]["volumes"][0]
		.as_str()
		.unwrap()
		.to_string();
	assert!(
		volume.starts_with('/') && volume.ends_with("/data:/data"),
		"{volume}"
	);
	assert!(!volume.contains("/./"), "{volume}");
}

#[test]
fn paths_of_an_included_job_are_anchored_to_the_file_that_wrote_them() {
	let dir = tempfile::tempdir().unwrap();
	fs::create_dir(dir.path().join("sub")).unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"include: [sub/inc.yaml]\nservices:\n  web:\n    image: nginx\n",
	)
	.unwrap();
	fs::write(
		dir.path().join("sub/inc.yaml"),
		"jobs:\n  j:\n    image: x\n    volumes: ['./data:/data']\n    triggers: {manual: true}\n",
	)
	.unwrap();
	let json = config_json(dir.path(), "compose.yaml");
	let volume = json["jobs"]["j"]["volumes"][0]
		.as_str()
		.unwrap()
		.to_string();
	assert!(volume.contains("/sub/"), "anchored to sub/: {volume}");
}

#[test]
fn a_job_joins_the_default_network_like_a_service() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"jobs:\n  j:\n    image: x\n    triggers: {manual: true}\n",
	)
	.unwrap();
	let json = config_json(dir.path(), "compose.yaml");
	assert_eq!(json["jobs"]["j"]["networks"][0], "default");
	assert!(json["networks"]["default"].is_object(), "{json}");
}

#[test]
fn a_host_mode_on_a_job_is_surfaced_by_config() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"jobs:\n  risky:\n    image: x\n    privileged: true\n    triggers: {manual: true}\n",
	)
	.unwrap();
	let out = podup(dir.path(), "compose.yaml", &["config", "--quiet"]);
	assert!(
		text(&out.stderr).contains("job \"risky\"") && text(&out.stderr).contains("privileged"),
		"{}",
		text(&out.stderr)
	);
}

#[test]
fn an_unknown_key_in_a_job_is_warned_about_and_not_echoed_back() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"jobs:\n  j:\n    image: x\n    restar: always\n    triggers: {manual: true}\n",
	)
	.unwrap();
	let out = podup(dir.path(), "compose.yaml", &["config"]);
	assert!(out.status.success(), "{}", text(&out.stderr));
	assert!(
		text(&out.stderr).contains("restar"),
		"{}",
		text(&out.stderr)
	);
	assert!(
		!text(&out.stdout).contains("restar"),
		"{}",
		text(&out.stdout)
	);
}

#[test]
fn a_job_extends_a_job_of_another_file() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("base.yaml"),
		"jobs:\n  base:\n    image: from-the-other-file\n    triggers: {manual: true}\n",
	)
	.unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"jobs:\n  j:\n    extends: {file: base.yaml, service: base}\n    triggers: {manual: true}\n",
	)
	.unwrap();
	let json = config_json(dir.path(), "compose.yaml");
	assert_eq!(json["jobs"]["j"]["image"], "from-the-other-file");
}

#[test]
fn a_job_cannot_extend_a_service_of_another_file() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("base.yaml"),
		"services:\n  base:\n    image: svc\n",
	)
	.unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"jobs:\n  j:\n    extends: {file: base.yaml, service: base}\n    triggers: {manual: true}\n",
	)
	.unwrap();
	let out = podup(dir.path(), "compose.yaml", &["config"]);
	assert!(!out.status.success());
	assert!(
		text(&out.stderr).contains("job 'base' not found in"),
		"{}",
		text(&out.stderr)
	);
}

#[test]
fn a_service_cannot_extend_a_job_of_another_file() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("base.yaml"),
		"jobs:\n  base:\n    image: job\n    triggers: {manual: true}\n",
	)
	.unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"services:\n  s:\n    extends: {file: base.yaml, service: base}\n",
	)
	.unwrap();
	let out = podup(dir.path(), "compose.yaml", &["config"]);
	assert!(!out.status.success());
	assert!(
		text(&out.stderr).contains("service 'base' not found in"),
		"{}",
		text(&out.stderr)
	);
}

#[test]
fn config_services_lists_services_and_not_jobs() {
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("compose.yaml"),
		"services:\n  web:\n    image: nginx\njobs:\n  j:\n    image: x\n    triggers: {manual: true}\n",
	)
	.unwrap();
	let out = podup(dir.path(), "compose.yaml", &["config", "--services"]);
	let listed = text(&out.stdout);
	assert!(listed.contains("web") && !listed.contains('j'), "{listed}");
}
