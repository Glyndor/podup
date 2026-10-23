use super::*;
#[test]
fn install_at_replaces_contents() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old version").unwrap();
	install_at(&target, b"new version").unwrap();
	assert_eq!(std::fs::read(&target).unwrap(), b"new version");
}
#[test]
fn install_at_creates_when_absent() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	install_at(&target, b"fresh").unwrap();
	assert_eq!(std::fs::read(&target).unwrap(), b"fresh");
}
#[cfg(unix)]
#[test]
fn install_at_preserves_executable_mode() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").unwrap();
	std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
	install_at(&target, b"new").unwrap();
	let mode = std::fs::metadata(&target).unwrap().permissions().mode();
	assert_eq!(mode & 0o777, 0o755);
}
#[cfg(unix)]
#[test]
fn install_at_strips_setuid_from_target_mode() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").unwrap();
	// A tampered/setuid target must not propagate its special bits onto the
	// freshly installed binary.
	std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o4755)).unwrap();
	install_at(&target, b"new").unwrap();
	let mode = std::fs::metadata(&target).unwrap().permissions().mode();
	assert_eq!(mode & 0o7000, 0, "setuid/setgid/sticky must be stripped");
	assert_eq!(mode & 0o777, 0o755);
}
#[test]
fn install_at_leaves_no_temp_files() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	install_at(&target, b"data").unwrap();
	let leftovers: Vec<_> = std::fs::read_dir(dir.path())
		.unwrap()
		.filter_map(|e| e.ok())
		.filter(|e| e.file_name().to_string_lossy().contains("update-"))
		.collect();
	assert!(leftovers.is_empty(), "temp file left behind");
}
#[test]
fn install_at_fails_when_target_dir_is_missing() {
	// A target whose parent directory does not exist must fail (the sibling
	// temp cannot be created) and must not leave anything behind.
	let dir = tempfile::tempdir().unwrap();
	let missing = dir.path().join("no-such-subdir");
	let target = missing.join("podup");
	assert!(install_at(&target, b"data").is_err());
	assert!(!missing.exists(), "must not create the missing parent dir");
}
/// `install_at` copies the target's permission bits (with the setuid/setgid/
/// sticky mask) onto the staged file before the rename. With the L5 path
/// moving the target aside first, `write_temp` had nothing to read and the
/// mode silently fell back to `0o755` for every operator who had chosen a
/// different one (`#1898`). Looping over `0o700` and `0o750` pins the two
/// restrictive modes a security-conscious operator is most likely to choose.
#[cfg(unix)]
#[test]
fn the_swap_keeps_a_restrictive_target_mode() {
	use std::os::unix::fs::PermissionsExt;
	for mode in [0o700, 0o750] {
		let dir = tempfile::tempdir().expect("tempdir");
		let target = dir.path().join("podup");
		std::fs::write(&target, b"old").expect("write target");
		std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode)).expect("set mode");
		let backup = move_target_aside(&target).expect("the move-aside must succeed");
		install_at(&target, b"new").expect("the install must succeed");
		assert_eq!(
			std::fs::read(&target).expect("read target"),
			b"new",
			"the staged bytes must be in place at the target"
		);
		assert_eq!(
			std::fs::metadata(&target)
				.expect("metadata target")
				.permissions()
				.mode() & 0o7777,
			mode,
			"the operator's mode {mode:o} must survive the update"
		);
		assert_eq!(
			std::fs::read(&backup).expect("read backup"),
			b"old",
			"the backup must still hold the pre-swap bytes"
		);
	}
}
/// #1360 (L5): `install_at` keeps the old in-place semantics, where the
/// target is replaced atomically. The L5 swap path adds a `move_target_aside`
/// *before* `install_at`, but the public surface (`install_at`) is the
/// building block tested here. The new `.old` sibling should not leak
/// to a directory that did not previously have one: the `.old` belongs
/// to the L5 caller, not to `install_at`.
#[test]
fn install_at_does_not_leave_an_old_sibling() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").unwrap();
	install_at(&target, b"new").unwrap();
	let old_sibling = dir.path().join("podup.old");
	assert!(
		!old_sibling.exists(),
		"install_at must not create a .old sibling; that is the L5 caller's job"
	);
}
/// #1367 (L5): the chmod-0000 case the issue's test plan calls out. A binary
/// the operator can no longer read or execute is the canonical "I have a
/// backup question for the rollback" case: the new bytes land on disk, the
/// self-test cannot even spawn the binary, and the L5 path must leave the
/// previous binary in place rather than leave the user with a half-installed
/// release. Simulate the full `install_binary` flow with a stub target: the
/// helper functions compose exactly the way the real function does, the
/// self-test fails for a chmod-0000 file (PermissionDenied at spawn), and
/// `restore_from_backup` puts the previous bytes back.
#[cfg(unix)]
#[test]
fn install_binary_rolls_back_when_the_target_is_unreadable() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"the previous binary").unwrap();

	let backup = move_target_aside(&target).expect("the move-aside must succeed");
	// The "new bytes" landing on disk: a real, fresh, chmod-0000 file the
	// kernel will refuse to exec, so the self-test cannot pass.
	// The swap is a rename in production, never an in-place write.
	install_at(&target, b"the new binary").unwrap();
	std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o000)).unwrap();

	// Spawning a chmod-0000 file is a hard PermissionDenied: the self-test
	// must surface that as a failure, and the rollback must restore the
	// previous binary. Mirrors the real `install_binary` failure path.
	let err = self_test(&target, "9.9.9").expect_err("a chmod-0000 binary must fail its self-test");
	assert!(
		format!("{err}").contains("could not run"),
		"the error should be a spawn failure, got: {err}"
	);
	restore_from_backup(&target, &backup).expect("the rollback must succeed");
	assert_eq!(
		std::fs::read(&target).unwrap(),
		b"the previous binary",
		"the rollback must put the previous binary back at the target"
	);
	// The .old sibling is consumed by the rollback.
	assert!(
		!backup.exists(),
		"the .old sibling is consumed by the rollback"
	);
}
