//! The capped readers against paths that are not ordinary files: links, device
//! nodes, sockets, directories and bytes that are not text. Each must be
//! refused or fail with an error, and none may block or read without end.

use std::os::unix::net::UnixListener;
use std::sync::mpsc;
use std::time::Duration;

use super::{read_capped, read_to_string_capped};

/// Run `f` on a thread and fail when it has not returned in 10 seconds: the
/// failure under test is a read that never ends.
fn within_budget<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
	let (tx, rx) = mpsc::channel();
	std::thread::spawn(move || {
		let _ = tx.send(f());
	});
	rx.recv_timeout(Duration::from_secs(10))
		.expect("the read did not return within 10 seconds")
}

#[test]
fn a_symlink_to_a_device_that_never_ends_is_refused_not_read() {
	let dir = tempfile::tempdir().expect("tempdir");
	let link = dir.path().join("zero");
	std::os::unix::fs::symlink("/dev/zero", &link).expect("symlink");
	let (a, b) = (link.clone(), link);
	assert!(within_budget(move || read_capped(a)).is_err());
	assert!(within_budget(move || read_to_string_capped(b)).is_err());
}

#[test]
fn a_device_a_socket_and_a_directory_are_refused_by_both_readers() {
	let dir = tempfile::tempdir().expect("tempdir");
	let sock = dir.path().join("s");
	let _listener = UnixListener::bind(&sock).expect("bind");
	let subdir = dir.path().join("d");
	std::fs::create_dir(&subdir).expect("mkdir");
	for path in [sock, subdir, "/dev/zero".into(), "/dev/null".into()] {
		let (a, b) = (path.clone(), path.clone());
		let bytes = within_budget(move || read_capped(a));
		let text = within_budget(move || read_to_string_capped(b));
		assert!(bytes.is_err(), "{} was read as bytes", path.display());
		assert!(text.is_err(), "{} was read as text", path.display());
	}
}

#[test]
fn a_symlink_to_an_ordinary_file_is_refused_so_the_reader_never_follows_one() {
	let dir = tempfile::tempdir().expect("tempdir");
	std::fs::write(dir.path().join("real"), "A=1\n").expect("write");
	std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).expect("link");
	let err = read_to_string_capped(dir.path().join("link")).expect_err("a link must be refused");
	assert!(err.to_string().contains("non-regular file"), "{err}");
}

#[test]
fn bytes_that_are_not_text_end_in_an_error_not_a_panic() {
	let dir = tempfile::tempdir().expect("tempdir");
	let path = dir.path().join("bin");
	std::fs::write(&path, b"A=\xff\xfe\x00\x80\n").expect("write");
	assert!(read_to_string_capped(&path).is_err());
	assert_eq!(read_capped(&path).expect("bytes"), b"A=\xff\xfe\x00\x80\n");
}

#[test]
fn a_path_through_a_missing_directory_or_a_file_is_an_error() {
	let dir = tempfile::tempdir().expect("tempdir");
	std::fs::write(dir.path().join("f"), "x").expect("write");
	assert!(read_capped(dir.path().join("f/inside")).is_err());
	assert!(read_capped(dir.path().join("nope/inside")).is_err());
	assert!(read_capped("").is_err());
}

/// The type is checked from the handle that was opened, and the open does not
/// wait for a writer. A pipe put under the name after the check of its type is
/// refused at once instead of holding the read, and the project lock with it, for
/// as long as nobody opens the other end.
#[test]
fn opening_a_pipe_that_has_no_writer_is_refused_and_does_not_wait() {
	let dir = tempfile::tempdir().expect("tempdir");
	let pipe = dir.path().join("pipe");
	rustix::fs::mknodat(
		rustix::fs::CWD,
		&pipe,
		rustix::fs::FileType::Fifo,
		rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
		0,
	)
	.expect("mkfifo");
	let a = pipe.clone();
	assert!(within_budget(move || super::open_regular(&a)).is_err());
	// An ordinary file is still opened.
	let file = dir.path().join("file");
	std::fs::write(&file, b"x").expect("write");
	assert!(within_budget(move || super::open_regular(&file)).is_ok());
}
