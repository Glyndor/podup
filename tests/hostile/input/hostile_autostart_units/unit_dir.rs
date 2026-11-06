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
	dir
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
	run(command(dir, &all))
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

#[test]
fn a_link_to_a_pipe_is_refused_not_followed() {
	let dir = project();
	let target = dir.path().join("elsewhere");
	mkfifo(&target);
	let link = format!("{PROJECT}-link.container");
	std::os::unix::fs::symlink(&target, units(dir.path()).join(&link)).unwrap();
	let outcome = autostart(dir.path(), &["install", "--mode", "quadlet", "--no-start"]);
	assert_refused("install over a link to a pipe", &outcome, &link);
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
