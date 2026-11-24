//! The unit does not replace a secret that is not the project's, against a real
//! Podman. `up` refuses to overwrite a secret that does not carry
//! `podup.project=<project>`; the unit's command does the same, or the first
//! `systemctl start` after a name clash would destroy a secret podup never made.

use std::path::Path;
use std::process::Command;

use super::real_tests::{exec_lines, podman_available};
use super::tests::FILE_SECRET;
use crate::compose::parse_str;
use crate::quadlet::generate_at;

fn podman(args: &[&str]) -> std::process::Output {
	Command::new("podman").args(args).output().unwrap()
}

fn secret_id(name: &str) -> String {
	String::from_utf8_lossy(
		&podman(&["secret", "inspect", "--format", "{{.ID}}", "--", name]).stdout,
	)
	.trim()
	.to_string()
}

fn secret_label(name: &str) -> String {
	String::from_utf8_lossy(
		&podman(&[
			"secret",
			"inspect",
			"--format",
			"{{index .Spec.Labels \"podup.project\"}}",
			"--",
			name,
		])
		.stdout,
	)
	.trim()
	.to_string()
}

struct Secret(String);

impl Drop for Secret {
	fn drop(&mut self) {
		let _ = podman(&["secret", "rm", "--", &self.0]);
	}
}

#[test]
fn the_unit_replaces_only_a_secret_that_is_the_projects() {
	if !podman_available() {
		eprintln!(
			"SKIP: the_unit_replaces_only_a_secret_that_is_the_projects: podman is not installed"
		);
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("token.txt");
	std::fs::write(&file, "from-the-file").unwrap();
	let project = format!("t{}guard", std::process::id());
	let yaml = FILE_SECRET.replace("./token.txt", file.to_str().unwrap());
	let out = generate_at(&parse_str(&yaml).unwrap(), &project, Path::new("/srv/app"));
	let unit = out
		.units
		.iter()
		.find(|u| u.filename.ends_with("podup-secrets.service"))
		.unwrap();
	let lines = exec_lines(&unit.contents);
	assert_eq!(lines.len(), 1);
	let run = || {
		let mut args = lines[0].iter();
		Command::new(args.next().unwrap())
			.args(args)
			.output()
			.unwrap()
	};
	let name = format!("{project}_secret_token");
	let _cleanup = Secret(name.clone());
	let decoy = dir.path().join("decoy.txt");
	std::fs::write(&decoy, "somebody-elses").unwrap();
	let decoy = decoy.to_str().unwrap();

	// Nothing there: created, with the project's label.
	let first = run();
	assert!(first.status.success());
	assert_eq!(secret_label(&name), project);

	// The project's own: replaced, a new secret under the same name.
	let before = secret_id(&name);
	let second = run();
	assert!(second.status.success());
	assert_ne!(
		secret_id(&name),
		before,
		"the project's own secret was not replaced"
	);

	// Another project's, and one with no label at all: the unit fails and what
	// was there is untouched.
	for label in [Some("another-project"), None] {
		assert!(podman(&["secret", "rm", &name]).status.success());
		let mut create = vec!["secret", "create"];
		let labelled = format!("podup.project={}", label.unwrap_or(""));
		if label.is_some() {
			create.extend(["--label", &labelled]);
		}
		create.extend([name.as_str(), decoy]);
		assert!(podman(&create).status.success());
		let id = secret_id(&name);
		let refused = run();
		assert!(
			!refused.status.success(),
			"the unit replaced a secret labelled"
		);
		assert!(String::from_utf8_lossy(&refused.stderr).contains("not replacing it"));
		assert_eq!(secret_id(&name), id, "the other secret was replaced");
		assert_eq!(secret_label(&name), label.unwrap_or(""));
	}
}

/// A name or a path that starts with a dash is a value, not an option: the script
/// ends the options of each `podman` call with `--`. The name `--help` would
/// print the help text and create nothing, and the path `-/x` would be an unknown
/// flag, without it.
#[test]
fn a_name_or_a_path_that_starts_with_a_dash_is_a_value() {
	if !podman_available() {
		eprintln!(
			"SKIP: a_name_or_a_path_that_starts_with_a_dash_is_a_value: podman is not installed"
		);
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	std::fs::create_dir(dir.path().join("-")).unwrap();
	std::fs::write(dir.path().join("-").join("x"), "dash-path").unwrap();
	let project = format!("t{}dash", std::process::id());
	let out = generate_at(
		&parse_str(FILE_SECRET).unwrap(),
		&project,
		Path::new("/srv/app"),
	);
	let unit = out
		.units
		.iter()
		.find(|u| u.filename.ends_with("podup-secrets.service"))
		.unwrap();
	let words = exec_lines(&unit.contents).remove(0);
	// /bin/sh, -c, script, argv0, name, project, path, mode
	assert_eq!(words.len(), 8);
	let _cleanup = Secret("--help".to_string());
	let _cleanup_name = Secret(format!("{project}_dash"));
	for name in ["--help", &format!("{project}_dash")] {
		let run = Command::new(&words[0])
			.args(&words[1..4])
			.args([name, &project, "-/x", ""])
			.current_dir(dir.path())
			.output()
			.unwrap();
		assert!(run.status.success());
		assert_eq!(secret_label(name), project);
	}
}

/// The unit's command for `token.txt`, generated while the file is `mode`, and a
/// way to run it.
fn generated_command(
	dir: &Path,
	project: &str,
	mode: u32,
) -> (std::path::PathBuf, Vec<String>, Secret) {
	use std::os::unix::fs::PermissionsExt;
	let file = dir.join("token.txt");
	std::fs::write(&file, "the-bytes").unwrap();
	std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
	let yaml = FILE_SECRET.replace("./token.txt", file.to_str().unwrap());
	let out = generate_at(&parse_str(&yaml).unwrap(), project, Path::new("/srv/app"));
	let unit = out
		.units
		.iter()
		.find(|u| u.filename.ends_with("podup-secrets.service"))
		.unwrap();
	let words = exec_lines(&unit.contents).remove(0);
	(file, words, Secret(format!("{project}_secret_token")))
}

fn run_command(words: &[String]) -> std::process::Output {
	Command::new(&words[0]).args(&words[1..]).output().unwrap()
}

/// What puts something else where the file was.
type Swap = Box<dyn Fn()>;

fn exists(name: &str) -> bool {
	podman(&["secret", "exists", "--", name]).status.success()
}

/// Between the generation and the start the file can be replaced by a link, a
/// directory or a pipe. `up` refuses all of them; the command does too, before it
/// asks Podman for anything, and creates nothing.
#[test]
fn the_command_refuses_what_up_refuses_where_the_file_was() {
	use std::os::unix::fs::symlink;
	if !podman_available() {
		eprintln!("SKIP: the_command_refuses_what_up_refuses_where_the_file_was: no podman");
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let project = format!("t{}kinds", std::process::id());
	let (file, words, secret) = generated_command(dir.path(), &project, 0o600);
	let folder = dir.path().join("folder");
	std::fs::create_dir(&folder).unwrap();
	let real = dir.path().join("real.txt");
	std::fs::write(&real, "elsewhere").unwrap();
	let replacements: [(&str, Swap); 5] = [
		(
			"link to /etc/shadow",
			Box::new({
				let f = file.clone();
				move || symlink("/etc/shadow", &f).unwrap()
			}),
		),
		(
			"link to a file",
			Box::new({
				let (f, r) = (file.clone(), real.clone());
				move || symlink(&r, &f).unwrap()
			}),
		),
		(
			"link to a directory",
			Box::new({
				let (f, d) = (file.clone(), folder.clone());
				move || symlink(&d, &f).unwrap()
			}),
		),
		(
			"directory",
			Box::new({
				let f = file.clone();
				move || std::fs::create_dir(&f).unwrap()
			}),
		),
		(
			"pipe",
			Box::new({
				let f = file.clone();
				move || {
					assert!(Command::new("mkfifo").arg(&f).status().unwrap().success());
				}
			}),
		),
	];
	for (_, put) in replacements {
		let _ = std::fs::remove_file(&file);
		let _ = std::fs::remove_dir(&file);
		put();
		let run = run_command(&words);
		let stderr = String::from_utf8_lossy(&run.stderr);
		assert!(!run.status.success(), "the command succeeded");
		assert!(stderr.contains("is not a regular file"));
		assert!(!exists(&secret.0), "a secret was created");
	}
	// And the file, back as it was, is made.
	let _ = std::fs::remove_file(&file);
	let _ = std::fs::remove_dir(&file);
	std::fs::write(&file, "the-bytes").unwrap();
	std::fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
	let run = run_command(&words);
	assert!(run.status.success());
	assert!(exists(&secret.0));
}

/// The units are generated while the file is 0600, and the container is given
/// `mode=600`. If the file has another mode when the unit starts, in either
/// direction, the command fails and creates nothing: a more open file is not what
/// was reviewed, and a more closed one would be mounted more open than `up` would
/// mount it. Generating the units again takes the new mode.
#[test]
fn the_command_refuses_a_file_whose_mode_is_not_the_one_it_had_when_the_units_were_generated() {
	use std::os::unix::fs::PermissionsExt;
	if !podman_available() {
		eprintln!("SKIP: the_command_refuses_a_file_whose_mode_is_not_the_one_it_had: no podman");
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let project = format!("t{}open", std::process::id());
	let (file, words, secret) = generated_command(dir.path(), &project, 0o600);
	let set = |mode: u32| {
		std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
	};
	for changed in [0o644, 0o640, 0o602, 0o700, 0o4600, 0o400] {
		set(changed);
		let run = run_command(&words);
		let stderr = String::from_utf8_lossy(&run.stderr);
		assert!(!run.status.success(), "the command succeeded");
		assert!(stderr.contains("and had 0600 when the units were generated"));
		assert!(!exists(&secret.0), "a secret was created");
	}
	set(0o600);
	let run = run_command(&words);
	assert!(run.status.success());
	assert!(exists(&secret.0));
	// Generating again, with the file at 0644, takes that mode.
	set(0o644);
	let yaml = FILE_SECRET.replace("./token.txt", file.to_str().unwrap());
	let out = generate_at(&parse_str(&yaml).unwrap(), &project, Path::new("/srv/app"));
	let unit = out
		.units
		.iter()
		.find(|u| u.filename.ends_with("podup-secrets.service"))
		.unwrap();
	let again = exec_lines(&unit.contents).remove(0);
	assert_eq!(again.last().unwrap(), "0644");
	let run = run_command(&again);
	assert!(run.status.success());
}

/// A copy of the real `podman` or `stat` that, the first time it is asked for what
/// the test names, first puts something else where the file was, and then runs the
/// real one. The unit's command finds programs through `PATH`, so this stands in
/// for somebody who changes the file while the command runs.
fn interposed(dir: &Path, program: &str, only_when: &str, swap: &str) -> std::path::PathBuf {
	use std::os::unix::fs::PermissionsExt;
	let real = String::from_utf8_lossy(
		&Command::new("sh")
			.args(["-c", &format!("command -v {program}")])
			.output()
			.unwrap()
			.stdout,
	)
	.trim()
	.to_string();
	let bin = dir.join("bin");
	std::fs::create_dir_all(&bin).unwrap();
	let script = bin.join(program);
	// `swap` runs once: a marker file says it has.
	std::fs::write(
		&script,
		format!(
			"#!/bin/sh\ncase \"$*\" in *'{only_when}'*) [ -e '{m}' ] || {{ : > '{m}'; {swap}; }};; esac\nexec {real} \"$@\"\n",
			m = dir.join(format!("{program}.done")).display()
		),
	)
	.unwrap();
	std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
	bin
}

fn run_with_path(words: &[String], bin: &Path) -> std::process::Output {
	let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
	Command::new(&words[0])
		.args(&words[1..])
		.env("PATH", path)
		.output()
		.unwrap()
}

/// The file is opened once and the secret is made from that descriptor. While the
/// command is asking Podman whether the secret exists, which is after the file was
/// opened and checked, the path is made a link to other bytes: the secret still
/// holds the bytes of the file that was opened.
#[test]
fn the_secret_is_made_from_the_file_that_was_opened_not_from_the_path() {
	if !podman_available() {
		eprintln!("SKIP: the_secret_is_made_from_the_file_that_was_opened: no podman");
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let project = format!("t{}once", std::process::id());
	let (file, words, secret) = generated_command(dir.path(), &project, 0o600);
	std::fs::write(&file, "the-bytes-that-were-checked").unwrap();
	std::fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
	let decoy = dir.path().join("decoy");
	std::fs::write(&decoy, "the-bytes-of-another-file").unwrap();
	let swap = format!(
		"mv '{f}' '{f}.moved' && ln -s '{d}' '{f}'",
		f = file.display(),
		d = decoy.display()
	);
	let bin = interposed(dir.path(), "podman", "secret exists", &swap);
	let run = run_with_path(&words, &bin);
	assert!(run.status.success());
	assert!(
		std::fs::symlink_metadata(&file)
			.unwrap()
			.file_type()
			.is_symlink(),
		"the path was not swapped, so nothing was tested"
	);
	let bytes = podman(&[
		"secret",
		"inspect",
		"--showsecret",
		"--format",
		"{{.SecretData}}",
		"--",
		&secret.0,
	]);
	assert_eq!(
		String::from_utf8_lossy(&bytes.stdout).trim(),
		"the-bytes-that-were-checked"
	);
}

/// The path is made a link to another file after it was opened and before the
/// command compares the open file with the path: it is not the same file any more,
/// and the command fails and creates nothing.
#[test]
fn a_path_that_is_not_the_file_that_was_opened_makes_the_command_fail() {
	if !podman_available() {
		eprintln!("SKIP: a_path_that_is_not_the_file_that_was_opened: no podman");
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let project = format!("t{}swap", std::process::id());
	let (file, words, secret) = generated_command(dir.path(), &project, 0o600);
	let decoy = dir.path().join("decoy");
	std::fs::write(&decoy, "elsewhere").unwrap();
	// The first `stat` that names the path without following it is the comparison.
	let swap = format!(
		"mv '{f}' '{f}.moved' && ln -s '{d}' '{f}'",
		f = file.display(),
		d = decoy.display()
	);
	let bin = interposed(dir.path(), "stat", "%d:%i --", &swap);
	let run = run_with_path(&words, &bin);
	let stderr = String::from_utf8_lossy(&run.stderr);
	assert!(!run.status.success(), "the command succeeded");
	assert!(stderr.contains("is not the regular file it was when it was opened"));
	assert!(!exists(&secret.0));
}
