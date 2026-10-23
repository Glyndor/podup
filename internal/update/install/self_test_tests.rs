use super::*;

/// Write an executable stub script and return its path.
#[cfg(unix)]
fn write_stub(dir: &Path, name: &str, body: &str) -> PathBuf {
	use std::os::unix::fs::PermissionsExt;
	let p = dir.join(name);
	std::fs::write(&p, body).unwrap();
	std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
	p
}

#[cfg(unix)]
#[test]
fn self_test_passes_for_a_zero_exit_and_fails_otherwise() {
	let dir = tempfile::tempdir().unwrap();
	// A binary that exits 0 and reports the expected version passes; a
	// non-zero exit fails.
	let ok = write_stub(
		dir.path(),
		"ok",
		"#!/bin/sh\necho \"podup 9.9.9\"\nexit 0\n",
	);
	let bad = write_stub(dir.path(), "bad", "#!/bin/sh\nexit 1\n");
	assert!(self_test(&ok, "9.9.9").is_ok());
	assert!(self_test(&bad, "9.9.9").is_err());
	// A non-executable / missing target is a spawn error, not a panic.
	assert!(self_test(&dir.path().join("nope"), "9.9.9").is_err());
}

/// The classification that silently did not work.
///
/// A real executable held open for writing cannot be run (the kernel
/// returns ETXTBSY) so this produces the genuine errno rather than a
/// hand-built error, and asserts the predicate the retry depends on. The
/// previous version of this check ran on an error already formatted into a
/// `String`, where the errno no longer exists: it returned false every time,
/// the retry never fired, and the flake it was written to prevent stayed.
///
/// Linux only, deliberately. Whether a given kernel refuses to exec a file
/// held open for writing, and for a script whether the check lands on the
/// script or on its interpreter, is that kernel's business, verified on
/// Linux. Asserting it elsewhere tests the platform rather than the
/// classifier, and writing the test so it passes vacuously where ETXTBSY
/// never fires would repeat the mistake this whole change is about. The
/// retry itself stays on every Unix: it is a no-op where the errno does not
/// occur.
#[cfg(target_os = "linux")]
#[test]
fn a_binary_open_for_writing_is_classified_as_text_file_busy() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("held");
	std::fs::copy("/bin/sh", &target).expect("/bin/sh is copyable");
	{
		use std::os::unix::fs::PermissionsExt;
		std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
	}
	// Held across the spawn on purpose; dropping it closes the window.
	let _writer = std::fs::OpenOptions::new()
		.write(true)
		.open(&target)
		.unwrap();
	let err = std::process::Command::new(&target)
		.arg("-c")
		.arg("exit 0")
		.spawn()
		.expect_err("a binary open for writing must not be executable");
	assert!(
		is_text_file_busy(&err),
		"ETXTBSY must be recognised from the io::Error itself, got {err:?}"
	);
}

#[cfg(unix)]
#[test]
fn self_test_rejects_a_version_mismatch() {
	let dir = tempfile::tempdir().unwrap();
	// A genuinely-signed but *older* replayed release exits 0 yet reports the
	// wrong version, so the rollback gate must reject it.
	let p = write_stub(
		dir.path(),
		"older",
		"#!/bin/sh\necho \"podup 1.0.0\"\nexit 0\n",
	);
	let err = self_test(&p, "9.9.9").unwrap_err();
	let msg = format!("{err}");
	assert!(msg.contains("rollback"), "{msg}");
	// A `v`-prefixed report still matches its unprefixed expectation.
	let v = write_stub(
		dir.path(),
		"vprefixed",
		"#!/bin/sh\necho \"podup v9.9.9\"\nexit 0\n",
	);
	assert!(self_test(&v, "9.9.9").is_ok());
}
