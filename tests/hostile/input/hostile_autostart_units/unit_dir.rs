//! The Quadlet unit directory holds something that is not a unit file.
//!
//! The directory is `${XDG_CONFIG_HOME}/containers/systemd/`, which podup only
//! warns about when it is writable by the group. A pipe put there under the
//! name of a project's unit used to hang `autostart install`, `status` and
//! `uninstall` for good, with no output: the owner marker was read with a plain
//! open, and opening a pipe nobody writes to waits in the kernel. Each case runs
//! the real binary under the 15-second deadline of `file_process::run`, so a
//! command that hangs fails the test and does not hold the suite.

use std::fs;
use std::path::{Path, PathBuf};

use super::file_process::{command, run, Outcome};

/// The project name `file_process::command` passes with `-p`.
const PROJECT: &str = "h3";

fn project() -> tempfile::TempDir {
	let dir = tempfile::tempdir().expect("tempdir");
	fs::write(
		dir.path().join("compose.yaml"),
		"services:\n  web:\n    image: nginx\n",
	)
	.unwrap();
	fs::create_dir_all(units(dir.path())).unwrap();
	stub_manager(dir.path());
	dir
}

/// A `systemctl` and a `loginctl` of the test's own, found first on `PATH`, that
/// answer every call with success and the fixed text below. Whether the machine
/// has a user manager, and what it would say, must not decide the outcome: a
/// refusal that only shows because the manager fails afterwards is not a
/// refusal of the unit directory.
fn stub_manager(dir: &Path) {
	use std::os::unix::fs::PermissionsExt;
	let bin = dir.join("bin");
	fs::create_dir_all(&bin).unwrap();
	for (name, body) in [
		("systemctl", "echo \"$*\" >> \"$(dirname \"$0\")/calls\"\ncase \"$*\" in *is-enabled*) echo enabled;; *is-active*) echo active;; *LoadState*) echo LoadState=not-found;; esac\nexit 0\n"),
		("loginctl", "echo Linger=yes\nexit 0\n"),
	] {
		let path = bin.join(name);
		fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
		fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
	}
}

/// `file_process::isolated` points XDG_CONFIG_HOME at `<dir>/config`.
fn units(dir: &Path) -> PathBuf {
	dir.join("config/containers/systemd")
}

fn mkfifo(path: &Path) {
	assert!(std::process::Command::new("mkfifo")
		.arg(path)
		.status()
		.expect("mkfifo")
		.success());
}

fn autostart(dir: &Path, args: &[&str]) -> Outcome {
	let mut all = vec!["autostart"];
	all.extend_from_slice(args);
	let mut cmd = command(dir, &all);
	let path = std::env::var("PATH").unwrap_or_default();
	cmd.env("PATH", format!("{}:{path}", dir.join("bin").display()));
	run(cmd)
}

fn names(dir: &Path) -> Vec<String> {
	let mut out: Vec<String> = fs::read_dir(units(dir))
		.unwrap()
		.flatten()
		.map(|e| e.file_name().to_string_lossy().into_owned())
		.collect();
	out.sort();
	out
}

fn assert_refused(what: &str, outcome: &Outcome, entry: &str) {
	assert!(
		!outcome.timed_out,
		"{what}: the command hung:\n{}",
		outcome.text
	);
	assert!(!outcome.success, "{what}: it succeeded:\n{}", outcome.text);
	assert!(
		outcome.text.contains(entry),
		"{what}: the error does not name the entry:\n{}",
		outcome.text
	);
}

#[test]
fn install_stops_at_a_pipe_and_writes_nothing() {
	let dir = project();
	let pipe = format!("{PROJECT}-front.container");
	mkfifo(&units(dir.path()).join(&pipe));
	let outcome = autostart(dir.path(), &["install", "--mode", "quadlet", "--no-start"]);
	assert_refused("install", &outcome, &pipe);
	assert_eq!(
		names(dir.path()),
		[pipe],
		"a unit was written before the refusal"
	);
}

#[test]
fn a_dry_run_install_gives_the_same_answer() {
	let dir = project();
	let pipe = format!("{PROJECT}-front.container");
	mkfifo(&units(dir.path()).join(&pipe));
	let outcome = autostart(dir.path(), &["install", "--mode", "quadlet", "--dry-run"]);
	assert_refused("install --dry-run", &outcome, &pipe);
}

#[test]
fn status_stops_at_a_pipe() {
	let dir = project();
	let pipe = format!("{PROJECT}-front.container");
	fs::write(
		units(dir.path()).join(format!("{PROJECT}-web.container")),
		format!("# podup-owner: {PROJECT}\n[Container]\nImage=nginx\n"),
	)
	.unwrap();
	mkfifo(&units(dir.path()).join(&pipe));
	assert_refused("status", &autostart(dir.path(), &["status"]), &pipe);
}

#[test]
fn uninstall_stops_at_a_pipe_and_removes_nothing() {
	let dir = project();
	let pipe = format!("{PROJECT}-front.container");
	let own = format!("{PROJECT}-web.container");
	fs::write(
		units(dir.path()).join(&own),
		format!("# podup-owner: {PROJECT}\n[Container]\nImage=nginx\n"),
	)
	.unwrap();
	mkfifo(&units(dir.path()).join(&pipe));
	assert_refused("uninstall", &autostart(dir.path(), &["uninstall"]), &pipe);
	assert!(
		names(dir.path()).contains(&own),
		"the project's own unit was removed"
	);
}

/// A link to a pipe that is not a unit of this project is a neighbour: the
/// install writes its own units, succeeds, and leaves the link and the pipe as
/// they were.
#[test]
fn a_link_to_a_pipe_beside_the_units_is_left_alone() {
	let dir = project();
	let target = dir.path().join("elsewhere");
	mkfifo(&target);
	let link = units(dir.path()).join(format!("{PROJECT}-link.container"));
	std::os::unix::fs::symlink(&target, &link).unwrap();
	let outcome = autostart(dir.path(), &["install", "--mode", "quadlet", "--no-start"]);
	assert!(!outcome.timed_out, "install hung:\n{}", outcome.text);
	assert!(outcome.success, "install failed:\n{}", outcome.text);
	assert!(fs::symlink_metadata(&link)
		.unwrap()
		.file_type()
		.is_symlink());
	assert!(is_fifo(&target), "the pipe was changed");
	assert!(names(dir.path()).contains(&format!("{PROJECT}-web.container")));
}

/// A link at the very name the install writes is replaced, not followed: the
/// pipe it pointed at is never opened (opening it for writing would wait for a
/// reader) and stays a pipe.
#[test]
fn a_link_to_a_pipe_at_the_name_of_a_unit_is_replaced_not_followed() {
	let dir = project();
	let target = dir.path().join("elsewhere");
	mkfifo(&target);
	let unit = units(dir.path()).join(format!("{PROJECT}-web.container"));
	std::os::unix::fs::symlink(&target, &unit).unwrap();
	let outcome = autostart(dir.path(), &["install", "--mode", "quadlet", "--no-start"]);
	assert!(!outcome.timed_out, "install hung:\n{}", outcome.text);
	assert!(outcome.success, "install failed:\n{}", outcome.text);
	assert!(
		fs::symlink_metadata(&unit).unwrap().file_type().is_file(),
		"the link was not replaced by a file:\n{}",
		outcome.text
	);
	assert!(is_fifo(&target), "the pipe was changed");
}

fn is_fifo(path: &Path) -> bool {
	use std::os::unix::fs::FileTypeExt;
	fs::symlink_metadata(path).unwrap().file_type().is_fifo()
}

/// The control: the same pipe under another project's name is nobody's concern
/// here, so the scan ends. If this hangs, the others are not about the name.
#[test]
fn a_pipe_that_is_not_the_projects_does_not_hold_status() {
	let dir = project();
	mkfifo(&units(dir.path()).join("other-front.container"));
	let outcome = autostart(dir.path(), &["status"]);
	assert!(
		!outcome.timed_out,
		"status hung on a sibling's pipe:\n{}",
		outcome.text
	);
}

/// podup can publish a unit past the 16 MiB input limit (escaping doubles a
/// long label value). Its marker is on the first line, so `uninstall` must
/// still recognise it, stop its service and remove it, and `status` count it.
#[test]
fn an_own_unit_longer_than_the_input_limit_is_stopped_and_removed() {
	let dir = project();
	let unit = units(dir.path()).join(format!("{PROJECT}-web.container"));
	let mut text = format!("# podup-owner: {PROJECT}\n[Container]\nImage=nginx\n").into_bytes();
	while text.len() < 17 * 1024 * 1024 {
		text.extend_from_slice(b"# padding\n");
	}
	fs::write(&unit, text).unwrap();
	let status = autostart(dir.path(), &["status"]);
	assert!(status.success, "status failed:\n{}", status.text);
	assert!(
		!status.text.contains("cannot be proven"),
		"the large unit was not recognised:\n{}",
		status.text
	);
	let outcome = autostart(dir.path(), &["uninstall"]);
	assert!(
		!outcome.timed_out && outcome.success,
		"uninstall:\n{}",
		outcome.text
	);
	assert!(!unit.exists(), "the large unit was left behind");
	let calls = fs::read_to_string(dir.path().join("bin/calls")).unwrap_or_default();
	assert!(
		calls.contains(&format!("{PROJECT}-web.service")),
		"never stopped:\n{calls}"
	);
}
