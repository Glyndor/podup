//! The self-test reads the probe's stdout under a time and size bound.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::self_test;

fn write_stub(dir: &Path, name: &str, body: &str) -> PathBuf {
	use std::os::unix::fs::PermissionsExt;
	let p = dir.join(name);
	std::fs::write(&p, body).unwrap();
	std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
	p
}

#[test]
fn a_descendant_holding_the_pipe_does_not_delay_the_answer() {
	let dir = tempfile::tempdir().unwrap();
	let p = write_stub(
		dir.path(),
		"leaves_one",
		"#!/bin/sh\nsleep 4 &\necho \"podup 9.9.9\"\nexit 0\n",
	);
	let start = Instant::now();
	assert!(self_test(&p, "9.9.9").is_ok());
	assert!(
		start.elapsed() < Duration::from_secs(3),
		"the self-test waited {:?} on the descendant's pipe",
		start.elapsed()
	);
}

#[test]
fn a_descendant_holding_the_pipe_with_no_answer_is_an_error_not_a_hang() {
	let dir = tempfile::tempdir().unwrap();
	let p = write_stub(dir.path(), "silent", "#!/bin/sh\nsleep 14 &\nexit 0\n");
	let start = Instant::now();
	let err = self_test(&p, "9.9.9").unwrap_err();
	let took = start.elapsed();
	assert!(took < Duration::from_secs(13), "waited {took:?}");
	assert!(format!("{err}").contains("within 10s"), "{err}");
}

#[test]
fn an_endless_reply_is_read_only_up_to_the_cap() {
	let dir = tempfile::tempdir().unwrap();
	// Never ends a line and never stops writing: the read stops at the cap and
	// reports a mismatch instead of waiting for the end of a stream.
	let p = write_stub(dir.path(), "endless", "#!/bin/sh\nyes aaaa | tr -d '\\n'\n");
	let start = Instant::now();
	assert!(self_test(&p, "9.9.9").is_err());
	assert!(start.elapsed() < Duration::from_secs(9));
}
