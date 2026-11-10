//! A followed path is classified by the handle that is opened, and the open
//! does not wait for a writer on a FIFO.

use std::sync::mpsc;
use std::time::Duration;

use super::{open_following, open_regular, Followed};

fn kind(followed: &Followed) -> &'static str {
	match followed {
		Followed::File(_) => "file",
		Followed::Directory => "directory",
		Followed::Other => "other",
	}
}

#[test]
fn a_link_to_a_regular_file_is_a_file() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("target");
	std::fs::write(&target, b"x").unwrap();
	let link = dir.path().join("link");
	std::os::unix::fs::symlink(&target, &link).unwrap();
	assert_eq!(kind(&open_following(&link).unwrap()), "file");
}

#[test]
fn a_link_to_a_directory_is_a_directory() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("target");
	std::fs::create_dir(&target).unwrap();
	let link = dir.path().join("link");
	std::os::unix::fs::symlink(&target, &link).unwrap();
	assert_eq!(kind(&open_following(&link).unwrap()), "directory");
}

#[test]
fn a_dangling_link_is_an_error() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("link");
	std::os::unix::fs::symlink(dir.path().join("missing"), &link).unwrap();
	assert!(open_following(&link).is_err());
}

#[test]
fn a_link_to_a_fifo_with_no_writer_is_refused_without_waiting() {
	let dir = tempfile::tempdir().unwrap();
	let fifo = dir.path().join("fifo");
	mkfifo(&fifo);
	let link = dir.path().join("link");
	std::os::unix::fs::symlink(&fifo, &link).unwrap();

	let (tx, rx) = mpsc::channel();
	let path = link.clone();
	std::thread::spawn(move || {
		let _ = tx.send(kind(&open_following(&path).unwrap()).to_string());
	});
	let answer = rx.recv_timeout(Duration::from_secs(5));
	if answer.is_err() {
		// Release the blocked open so the thread does not outlive the test.
		let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
	}
	assert_eq!(answer.expect("the open waited for a writer"), "other");
}

/// Plant a FIFO with the `mkfifo` tool: rustix has no `mknodat` on macOS, std has
/// no `mkfifo`, and the crate forbids `unsafe`.
fn mkfifo(path: &std::path::Path) {
	assert!(std::process::Command::new("mkfifo")
		.arg(path)
		.status()
		.expect("mkfifo")
		.success());
}

#[test]
fn a_path_that_became_a_fifo_after_the_lookup_is_refused_without_waiting() {
	let dir = tempfile::tempdir().unwrap();
	let looked_up = dir.path().join("regular");
	std::fs::write(&looked_up, b"x").unwrap();
	let expected = std::fs::metadata(&looked_up).unwrap();
	let fifo = dir.path().join("fifo");
	mkfifo(&fifo);

	let (tx, rx) = mpsc::channel();
	let path = fifo.clone();
	std::thread::spawn(move || {
		let _ = tx.send(open_regular(&path, &expected).unwrap().is_none());
	});
	let answer = rx.recv_timeout(Duration::from_secs(5));
	if answer.is_err() {
		// Release the blocked open so the thread does not outlive the test.
		let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
	}
	assert!(
		answer.expect("the open waited for a writer"),
		"a FIFO was handed over as a file"
	);
}

#[test]
fn a_path_that_became_another_regular_file_is_refused() {
	let dir = tempfile::tempdir().unwrap();
	let looked_up = dir.path().join("a");
	let swapped = dir.path().join("b");
	std::fs::write(&looked_up, b"x").unwrap();
	std::fs::write(&swapped, b"y").unwrap();
	let expected = std::fs::metadata(&looked_up).unwrap();
	assert!(open_regular(&swapped, &expected).unwrap().is_none());
}

#[test]
fn the_regular_file_that_was_looked_up_is_handed_over() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("a");
	std::fs::write(&path, b"x").unwrap();
	let expected = std::fs::metadata(&path).unwrap();
	assert!(open_regular(&path, &expected).unwrap().is_some());
}
