//! The unit against the real tools: `systemd-analyze`, the Quadlet generator,
//! Podman, and a live systemd user manager. Each test skips by name, with a
//! `SKIP:` line, where its tool is missing.

use super::tests::{generate, secrets_unit, yaml_path, BASE, FILE_SECRET};
use crate::backends::quadlet::tests::generator::generator;
use crate::backends::quadlet::tests::real_env::{command, manager_unit_dir};
use crate::compose::parse_str;
use crate::quadlet::generate_at;
use std::path::Path;

/// The words of an `ExecStart=` line the way systemd reads it for the
/// shape written here: double-quoted words, `\\` and `\"` escapes, `%%` and
/// `$$` for the literal characters.
pub(super) fn words(line: &str) -> Vec<String> {
	let mut out = Vec::new();
	let mut chars = line.chars().peekable();
	while let Some(&c) = chars.peek() {
		if c == ' ' {
			chars.next();
		} else if c == '"' {
			chars.next();
			let mut word = String::new();
			while let Some(c) = chars.next() {
				match c {
					'"' => break,
					'\\' => word.push(chars.next().unwrap()),
					'%' | '$' => {
						assert_eq!(chars.next(), Some(c), "a lone in");
						word.push(c);
					}
					other => word.push(other),
				}
			}
			out.push(word);
		} else {
			let mut word = String::new();
			while let Some(&c) = chars.peek() {
				if c == ' ' {
					break;
				}
				word.push(c);
				chars.next();
			}
			out.push(word);
		}
	}
	out
}

pub(super) fn exec_lines(unit: &str) -> Vec<Vec<String>> {
	unit.lines()
		.filter_map(|l| l.strip_prefix("ExecStart="))
		.map(words)
		.collect()
}

/// Whether the tests that run `podman` may. They need the Podman the export is
/// written for (the floor): the 4.9 of the `ubuntu-latest` runner reads `podman
/// secret create` differently (it refuses a regular file on standard input, and
/// `--replace` fails for a secret that does not exist), so a test that ran it
/// would fail for a reason that is no fault of the code. The Quadlet generator
/// helper judges the version, skips with a line on stderr, and fails instead
/// where `PODUP_REQUIRE_QUADLET_GENERATOR=1` says a Podman at the floor is
/// guaranteed.
pub(super) fn podman_available() -> bool {
	crate::backends::quadlet::tests::generator::generator().is_some()
		&& command("podman").arg("--version").output().is_ok()
}

#[test]
fn systemd_accepts_the_unit() {
	let dir = tempfile::tempdir().unwrap();
	let out = generate(&FILE_SECRET.replace("./token.txt", "/srv/my dir/%h/$$HOME/t.txt"));
	let unit = secrets_unit(&out).unwrap();
	let path = dir.path().join("proj-podup-secrets.service");
	std::fs::write(&path, unit).unwrap();
	let Ok(run) = command("systemd-analyze")
		.args(["--user", "verify"])
		.arg(&path)
		.output()
	else {
		eprintln!("SKIP: systemd-analyze is not installed");
		return;
	};
	let text = String::from_utf8_lossy(&run.stderr);
	// A unit is judged by what is said about this file.
	assert!(
		!text.contains("proj-podup-secrets.service:"),
		"systemd-analyze complained"
	);
}

#[test]
fn the_generator_makes_the_container_wait_for_the_unit() {
	let Some(generator) = generator() else {
		return;
	};
	let dir = tempfile::tempdir().unwrap();
	let out = generate(FILE_SECRET);
	crate::quadlet::write_units(dir.path(), &out.units).unwrap();
	let run = command(&generator)
		.args(["-dryrun", "-user"])
		.env("QUADLET_UNIT_DIRS", dir.path())
		.output()
		.unwrap();
	let text = String::from_utf8_lossy(&run.stdout).into_owned();
	assert!(
		text.contains("Requires=proj-podup-secrets.service")
			&& text.contains("After=proj-podup-secrets.service")
	);
	assert!(text.contains("--secret proj_secret_token"));
}

/// The unit's own commands, run: the secret is made from the file, a
/// container reads it, a file that is gone makes the command fail, and a
/// container whose secret does not exist does not start.
#[test]
fn the_unit_creates_the_secret_and_a_missing_file_fails_closed() {
	if !podman_available() {
		eprintln!("SKIP: podman is not installed");
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("token.txt");
	std::fs::write(&file, "from-the-file").unwrap();
	let project = format!("t{}secrets", std::process::id());
	let yaml = FILE_SECRET.replace("./token.txt", file.to_str().unwrap());
	let out = generate_at(&parse_str(&yaml).unwrap(), &project, Path::new(BASE));
	let unit = out
		.units
		.iter()
		.find(|u| u.filename.ends_with("podup-secrets.service"))
		.unwrap();
	let scoped = format!("{project}_secret_token");
	let create = || {
		let lines = exec_lines(&unit.contents);
		assert_eq!(lines.len(), 1);
		let mut args = lines[0].iter();
		command(args.next().unwrap()).args(args).output().unwrap()
	};
	let cleanup = || {
		let _ = command("podman").args(["secret", "rm", &scoped]).output();
	};
	cleanup();
	let made = create();
	assert!(made.status.success());
	let read = command("podman")
		.args([
			"run",
			"--rm",
			"--secret",
			&scoped,
			"docker.io/library/busybox:latest",
			"cat",
		])
		.arg(format!("/run/secrets/{scoped}"))
		.output()
		.unwrap();
	assert_eq!(String::from_utf8_lossy(&read.stdout), "from-the-file");
	// The file is rotated: the next start of the unit makes the secret again
	// from the new bytes (`--replace`), and does not fail on the old one.
	std::fs::write(&file, "rotated").unwrap();
	let again = create();
	assert!(again.status.success());
	let reread = command("podman")
		.args([
			"run",
			"--rm",
			"--secret",
			&scoped,
			"docker.io/library/busybox:latest",
			"cat",
		])
		.arg(format!("/run/secrets/{scoped}"))
		.output()
		.unwrap();
	assert_eq!(String::from_utf8_lossy(&reread.stdout), "rotated");
	let labelled = command("podman")
		.args([
			"secret",
			"inspect",
			"--format",
			"{{index .Spec.Labels \"podup.project\"}}",
			&scoped,
		])
		.output()
		.unwrap();
	assert_eq!(String::from_utf8_lossy(&labelled.stdout).trim(), project);

	// The file goes away: the unit's command fails, so the containers that
	// require it are not started.
	std::fs::remove_file(&file).unwrap();
	let again = create();
	assert!(
		!again.status.success(),
		"the command succeeded with no file"
	);
	cleanup();
	// And a container that names a secret that does not exist does not start.
	let missing = command("podman")
		.args([
			"run",
			"--rm",
			"--secret",
			&scoped,
			"docker.io/library/busybox:latest",
			"true",
		])
		.output()
		.unwrap();
	assert!(!missing.status.success());
}

// --- a live systemd user manager --------------------------------------

fn limited(program: &str, args: &[&str], secs: u64) -> std::process::Output {
	let mut child = command(program)
		.args(args)
		.stdin(std::process::Stdio::null())
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.spawn()
		.unwrap();
	let started = std::time::Instant::now();
	loop {
		if child.try_wait().unwrap().is_some() {
			return child.wait_with_output().unwrap();
		}
		if started.elapsed().as_secs() > secs {
			let _ = child.kill();
			let _ = child.wait();
			panic!("{program} did not end in {secs} s");
		}
		std::thread::sleep(std::time::Duration::from_millis(50));
	}
}

fn systemctl(args: &[&str]) -> std::process::Output {
	let mut all = vec!["--user"];
	all.extend_from_slice(args);
	limited("systemctl", &all, 90)
}

fn text(out: &std::process::Output) -> String {
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// How a program ended and what it said about it, for a failure message.
fn said(out: &std::process::Output) -> String {
	format!(
		"{}: {}",
		out.status,
		String::from_utf8_lossy(&out.stderr).trim()
	)
}

/// Everything the live test puts on this machine, taken off again when it
/// ends, however it ends: the links in the user unit directory, the services,
/// the container and the secret.
struct Live {
	units: Vec<String>,
	container: String,
	secret: String,
}

impl Drop for Live {
	fn drop(&mut self) {
		for unit in self.units.iter().rev() {
			let _ = systemctl(&["stop", unit]);
		}
		let _ = limited("podman", &["rm", "-f", "-t", "0", &self.container], 60);
		let _ = limited("podman", &["secret", "rm", &self.secret], 60);
		// `systemctl link` put a symlink in the user unit directory, named as
		// the unit. It is found by name, not by asking systemd: the file it
		// points at may be gone by now, and then systemd no longer knows it.
		let linked_in = manager_unit_dir();
		for unit in &self.units {
			if let Some(dir) = &linked_in {
				let path = dir.join(unit);
				if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
					let _ = std::fs::remove_file(&path);
				}
			}
			let _ = systemctl(&["reset-failed", unit]);
		}
		let _ = systemctl(&["daemon-reload"]);
	}
}

/// With a systemd user manager and the Quadlet generator: install what the
/// export wrote, start the container with no secret file and see it stay
/// down with the secrets unit failed, then with the file and see it run with
/// the secret. The file sits under a directory whose name holds `%h` and
/// `$HOME`, so the same run shows that Podman receives the path as written.
#[test]
fn a_live_manager_keeps_the_container_down_until_the_file_exists() {
	let Some(generator) = generator() else {
		return;
	};
	if !podman_available() {
		eprintln!("SKIP: a_live_manager_keeps_the_container_down_until_the_file_exists: podman is not installed");
		return;
	}
	let alive = command("systemctl")
		.args(["--user", "show", "-p", "Version", "--value"])
		.output()
		.is_ok_and(|o| o.status.success() && !o.stdout.is_empty());
	if !alive {
		eprintln!("SKIP: a_live_manager_keeps_the_container_down_until_the_file_exists: `systemctl --user` does not answer");
		return;
	}
	let project = format!("t{}sec", std::process::id());
	let scoped = format!("{project}_secret_token");
	let web = format!("{project}-web.service");
	let secrets = format!("{project}-podup-secrets.service");
	let mut live = Live {
		units: Vec::new(),
		container: format!("{project}-web"),
		secret: scoped.clone(),
	};

	let dir = tempfile::tempdir().unwrap();
	let secret_dir = dir.path().join("50%h").join("$HOME");
	std::fs::create_dir_all(&secret_dir).unwrap();
	let file = secret_dir.join("token");
	let yaml = format!(
		"services:\n  web:\n    image: docker.io/library/busybox:latest\n    command: [sleep, '120']\n    secrets: [token]\nsecrets:\n  token:\n    file: {}\n",
		yaml_path(file.to_str().unwrap())
	);
	let out = generate_at(&parse_str(&yaml).unwrap(), &project, Path::new(BASE));
	let inputs = dir.path().join("units");
	let generated = dir.path().join("services");
	std::fs::create_dir_all(&generated).unwrap();
	crate::quadlet::write_units(&inputs, &out.units).unwrap();
	let _ = command(&generator)
		.args(["-user"])
		.arg(&generated)
		.env("QUADLET_UNIT_DIRS", &inputs)
		.output()
		.unwrap();
	assert!(generated.join(&web).exists());

	for path in [generated.join(&web), inputs.join(&secrets)] {
		let linked = systemctl(&["link", path.to_str().unwrap()]);
		assert!(
			linked.status.success(),
			"systemctl --user link {}: {}",
			path.display(),
			said(&linked)
		);
		live.units
			.push(path.file_name().unwrap().to_string_lossy().into_owned());
	}
	let reloaded = systemctl(&["daemon-reload"]);
	assert!(
		reloaded.status.success(),
		"systemctl --user daemon-reload: {}",
		said(&reloaded)
	);

	// No file: the secrets unit fails and the container does not start.
	let started = systemctl(&["start", &web]);
	assert!(
		!started.status.success(),
		"the container started with no secret file"
	);
	let state = systemctl(&["show", "-p", "ActiveState", "--value", &secrets]);
	assert_eq!(text(&state), "failed", "show: {}", said(&state));
	let active = systemctl(&["is-active", &web]);
	assert_ne!(text(&active), "active", "is-active: {}", said(&active));

	// The file exists: the secrets unit runs, the container starts, and the
	// secret holds the bytes of the file at the path as it was written.
	std::fs::write(&file, "live-secret").unwrap();
	let _ = systemctl(&["reset-failed", &secrets]);
	let started = systemctl(&["start", &web]);
	assert!(
		started.status.success(),
		"systemctl --user start: {}",
		said(&started)
	);
	let active = systemctl(&["is-active", &web]);
	assert_eq!(text(&active), "active", "is-active: {}", said(&active));
	let state = systemctl(&["show", "-p", "ActiveState", "--value", &secrets]);
	assert_eq!(text(&state), "active", "show: {}", said(&state));
	let read = limited(
		"podman",
		// The compose name, not the Podman name: where `up` mounts it.
		&["exec", &live.container, "cat", "/run/secrets/token"],
		60,
	);
	// The message holds neither what the container printed nor the secret.
	assert!(
		text(&read) == "live-secret",
		"podman exec did not show the bytes of the file: {}",
		read.status
	);
}
