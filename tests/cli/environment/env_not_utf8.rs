//! A variable in the environment whose bytes are not UTF-8 must not stop
//! podup. It cannot be interpolated into a compose file, so it is ignored;
//! reading the environment used to panic.

#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::process::Command;

#[test]
fn a_non_utf8_variable_is_ignored_not_a_panic() {
	let dir = tempfile::tempdir().expect("create temp dir");
	let file = dir.path().join("compose.yaml");
	std::fs::write(
		&file,
		"services:\n  web:\n    image: x\n    user: ${KEPT}\n",
	)
	.expect("write compose file");

	let out = Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["--socket", "unix:///nonexistent/x.sock"])
		.args(["-f", file.to_str().unwrap(), "config"])
		.env("KEPT", "app")
		.env("BROKEN", OsStr::from_bytes(b"\xff\xfe"))
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup");

	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!stderr.contains("internal error"),
		"podup panicked on a non-UTF-8 variable: {stderr}"
	);
	assert!(out.status.success(), "{stderr}");
	assert!(
		String::from_utf8_lossy(&out.stdout).contains("user: app"),
		"the readable variables must still interpolate"
	);
}
