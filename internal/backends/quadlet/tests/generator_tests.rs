//! Tests of the generator lookup, against stand-in executables.

use super::*;
use std::os::unix::fs::PermissionsExt;

const FLOOR: Version = (6, 2, 0);

/// An executable that prints `output` and exits with `code`.
fn fake(dir: &Path, output: &str, code: i32) -> PathBuf {
	let path = dir.join("quadlet");
	std::fs::write(&path, format!("#!/bin/sh\necho '{output}'\nexit {code}\n")).unwrap();
	std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
	path
}

fn run(paths: &[PathBuf], require: bool) -> (Option<PathBuf>, String) {
	let mut log = Vec::new();
	let found = resolve(paths, FLOOR, require, &mut log);
	(found, String::from_utf8(log).unwrap())
}

fn panic_message(paths: &[PathBuf], require: bool) -> String {
	let caught = std::panic::catch_unwind(|| run(paths, require)).unwrap_err();
	caught
		.downcast_ref::<String>()
		.cloned()
		.expect("the failure carries a message")
}

#[test]
fn a_generator_at_or_above_the_floor_is_returned() {
	for printed in ["6.2.0", "6.2.1", "6.10.0", "7.0.0", "6.3.0-dev", "v6.2.0"] {
		let dir = tempfile::tempdir().unwrap();
		let path = fake(dir.path(), printed, 0);
		for require in [false, true] {
			let (found, log) = run(std::slice::from_ref(&path), require);
			assert_eq!(found, Some(path.clone()), "{printed}");
			assert_eq!(log, "", "{printed}");
		}
	}
}

#[test]
fn an_old_generator_skips_without_the_switch_and_names_its_version() {
	let dir = tempfile::tempdir().unwrap();
	let path = fake(dir.path(), "4.9.3", 0);
	let (found, log) = run(&[path], false);
	assert_eq!(found, None);
	assert!(log.starts_with("SKIP: "), "{log}");
	assert!(log.contains("4.9.3") && log.contains("6.2.0"), "{log}");
}

#[test]
fn an_old_generator_fails_with_the_switch_and_names_its_version() {
	let dir = tempfile::tempdir().unwrap();
	let path = fake(dir.path(), "4.9.3", 0);
	let message = panic_message(&[path], true);
	assert!(message.contains(REQUIRE_VAR), "{message}");
	assert!(message.contains("4.9.3"), "{message}");
}

#[test]
fn an_unreadable_version_fails_even_without_the_switch() {
	for printed in ["", "quadlet", "6.2", "6.2.0.1", "six.two.zero"] {
		let dir = tempfile::tempdir().unwrap();
		let path = fake(dir.path(), printed, 0);
		for require in [false, true] {
			let message = panic_message(std::slice::from_ref(&path), require);
			assert!(message.contains("not a version"), "{printed:?}: {message}");
		}
	}
}

#[test]
fn a_generator_that_will_not_print_its_version_fails() {
	let dir = tempfile::tempdir().unwrap();
	let path = fake(dir.path(), "6.2.0", 3);
	let message = panic_message(&[path], false);
	assert!(message.contains("-version"), "{message}");
}

#[test]
fn an_absent_generator_skips_without_the_switch_and_fails_with_it() {
	let missing = vec![PathBuf::from("/nonexistent/podman/quadlet")];
	let (found, log) = run(&missing, false);
	assert_eq!(found, None);
	assert!(
		log.starts_with("SKIP: ") && log.contains("/nonexistent/podman/quadlet"),
		"{log}"
	);
	let message = panic_message(&missing, true);
	assert!(message.contains(REQUIRE_VAR), "{message}");
	assert!(message.contains("/nonexistent/podman/quadlet"), "{message}");
}

#[test]
fn the_first_path_that_is_a_file_is_the_one_judged() {
	let dir = tempfile::tempdir().unwrap();
	let old = fake(dir.path(), "4.9.3", 0);
	let other = tempfile::tempdir().unwrap();
	let new = fake(other.path(), "6.2.0", 0);
	let missing = PathBuf::from("/nonexistent/quadlet");
	let (found, _) = run(&[missing.clone(), old.clone(), new.clone()], false);
	assert_eq!(found, None);
	let (found, _) = run(&[missing, new.clone(), old], false);
	assert_eq!(found, Some(new));
}
