//! The mode a container sees for a secret or a config written in each form of
//! `mode:`, against a real Podman: each is the mode it spells.

use std::fs;

use super::*;

const FORMS: [(&str, &str); 6] = [
	("0o644", "644"),
	("0o404", "404"),
	("0o400", "400"),
	("0x100", "400"),
	("0400", "400"),
	("\"0644\"", "644"),
];

#[tokio::test]
async fn every_form_of_mode_is_the_mode_the_container_sees() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let mut secrets = String::new();
	let mut configs = String::new();
	let mut top_secrets = String::new();
	let mut top_configs = String::new();
	for (i, (form, _)) in FORMS.iter().enumerate() {
		fs::write(dir.path().join(format!("f{i}")), "x").unwrap();
		secrets.push_str(&format!("      - source: s{i}\n        mode: {form}\n"));
		configs.push_str(&format!("      - source: c{i}\n        mode: {form}\n"));
		top_secrets.push_str(&format!("  s{i}:\n    file: ./f{i}\n"));
		top_configs.push_str(&format!("  c{i}:\n    file: ./f{i}\n"));
	}
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		format!(
			"services:\n  web:\n    image: busybox:latest\n    command: [sleep, '120']\n    secrets:\n{secrets}    configs:\n{configs}secrets:\n{top_secrets}configs:\n{top_configs}"
		),
	)
	.unwrap();
	let file = compose.to_str().unwrap();
	let project = format!("t{}-modes", std::process::id());
	let up = run(&["-f", file, "-p", &project, "up", "-d"]);
	let stat = run(&[
		"-f",
		file,
		"-p",
		&project,
		"exec",
		"-T",
		"web",
		"sh",
		"-c",
		"for f in /run/secrets/s* /c*; do stat -c '%n %a' $f; done",
	]);
	run(&["-f", file, "-p", &project, "down", "-v", "-t", "0"]);
	assert!(up.status.success());
	let seen = String::from_utf8_lossy(&stat.stdout).to_string();
	for (i, (_, mode)) in FORMS.iter().enumerate() {
		for path in [format!("/run/secrets/s{i}"), format!("/c{i}")] {
			assert!(
				seen.contains(&format!("{path} {mode}\n")),
				"mode for should be"
			);
		}
	}
}

/// A stack with the modes a deployed project has, written in the forms that were
/// read the same before the change.
const DEPLOYED: &str = "services:\n  web:\n    image: busybox:latest\n    command: [sleep, '120']\n    secrets:\n      - source: a\n        mode: 0400\n      - source: b\n        mode: 0644\n      - source: c\n        mode: 400\n    configs:\n      - source: d\n        mode: 0440\n    volumes:\n      - type: tmpfs\n        target: /run/t1\n        tmpfs:\n          mode: 1777\n      - type: tmpfs\n        target: /run/t2\n        tmpfs:\n          mode: 700\nsecrets:\n  a: {file: ./f}\n  b: {file: ./f}\n  c: {file: ./f}\nconfigs:\n  d: {file: ./f}\n";

fn deployed_file(dir: &std::path::Path) -> String {
	fs::write(dir.join("f"), "x").unwrap();
	let compose = dir.join("docker-compose.yml");
	fs::write(&compose, DEPLOYED).unwrap();
	compose.to_str().unwrap().to_string()
}

/// The hash of the service, which the config hash of every container derives
/// from, is the one the binary before the change printed for the same file
/// (`1bdb8b4a…`, measured on `develop` at 5f4bc784): a stack deployed then is not
/// recreated now.
#[test]
fn the_service_hash_of_a_deployed_stack_is_the_one_it_had() {
	let dir = tempfile::tempdir().unwrap();
	let file = deployed_file(dir.path());
	let out = run_ok(&["-f", &file, "config", "--hash", "web"]);
	let text = String::from_utf8_lossy(&out.stdout);
	assert!(text.contains("web 1bdb8b4a246b87cb9d0cf8daa2254d3f0597d8c2f433347d9d0ab5a8448f0ac1"));
}

/// What `config` prints is meant to be fed back, and reads back as the same modes
/// with nothing refused or noticed, in YAML and in JSON.
#[test]
fn what_config_prints_reads_back_as_the_same_modes() {
	let dir = tempfile::tempdir().unwrap();
	let file = deployed_file(dir.path());
	let first = run_ok(&["-f", &file, "config"]);
	let printed = String::from_utf8_lossy(&first.stdout).to_string();
	assert!(printed.contains("mode: '0400'") || printed.contains("mode: \"0400\""));
	assert!(!printed.contains("mode: 420"));
	for (name, format) in [("again.yml", "yaml"), ("again.json", "json")] {
		let shown = run_ok(&["-f", &file, "config", "--format", format]);
		let again = dir.path().join(name);
		fs::write(&again, &shown.stdout).unwrap();
		let back = run_ok(&["-f", again.to_str().unwrap(), "config", "--format", format]);
		assert_eq!(
			shown.stdout, back.stdout,
			"the output changed when it was read back"
		);
		assert!(!String::from_utf8_lossy(&back.stderr).contains("mode"));
	}
}

const NOTICE: &str = "is written without a leading 0";

/// Two services that write the same tmpfs mode as an integer, a form that is very
/// common (`1777`, `700`).
const SAME_MODE_TWICE: &str = "services:\n  a:\n    image: busybox:latest\n    command: [sleep, '120']\n    volumes:\n      - type: tmpfs\n        target: /run/x\n        tmpfs:\n          mode: 700\n  b:\n    image: busybox:latest\n    command: [sleep, '120']\n    volumes:\n      - type: tmpfs\n        target: /run/y\n        tmpfs:\n          mode: 700\n          size: 1048576\n";

fn stderr_of(file: &str, args: &[&str]) -> String {
	let mut all = vec!["-f", file];
	all.extend_from_slice(args);
	let out = std::process::Command::new(bin())
		.args(&all)
		.env("PODMAN_SOCKET", "/nonexistent/podup-notice.sock")
		.stdin(std::process::Stdio::null())
		.output()
		.unwrap();
	String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The notice about an integer with no leading zero is said by the commands that
/// apply a mode or check the file, once for the mode however many services
/// write it, and by no other command.
#[test]
fn the_notice_is_said_by_the_commands_that_apply_or_check_and_by_no_other() {
	let dir = tempfile::tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(&compose, SAME_MODE_TWICE).unwrap();
	let file = compose.to_str().unwrap();

	for args in [
		&["config"][..],
		&["config", "--format", "json"],
		&["audit"],
		&["generate", "quadlet"],
		&["up", "-d"],
		&["create"],
		&["run", "--rm", "a", "true"],
	] {
		let said = stderr_of(file, args);
		assert_eq!(said.matches(NOTICE).count(), 1);
	}
	for args in [
		&["ps"][..],
		&["logs"],
		&["restart"],
		&["stop"],
		&["start"],
		&["down"],
		&["exec", "a", "true"],
		&["top"],
		&["port", "a", "80"],
		&["images"],
		&["kill"],
		&["pause"],
	] {
		let said = stderr_of(file, args);
		assert!(!said.contains(NOTICE));
	}
}

/// A second distinct mode is a second notice.
#[test]
fn a_notice_is_said_once_for_each_mode() {
	let dir = tempfile::tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		SAME_MODE_TWICE.replace("mode: 700\n          size", "mode: 1777\n          size"),
	)
	.unwrap();
	let said = stderr_of(compose.to_str().unwrap(), &["config"]);
	assert_eq!(said.matches(NOTICE).count(), 2);
}
