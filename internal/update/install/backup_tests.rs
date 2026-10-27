use super::*;

/// `move_target_aside` always leaves a recoverable `.old` sibling on disk so
/// the self-test can roll back if it fails. On Unix the target itself is
/// also expected to still be in place (the swap renames the staged file over
/// its inode atomically): moving it aside would orphan the path between the
/// snapshot and the swap, and `write_temp` reads its permission bits from the
/// live target (`#1898`). On Windows a running `.exe` cannot be overwritten,
/// so it has to be moved aside by `move_target_aside` instead.
#[test]
fn move_target_aside_leaves_the_old_binary_on_disk_for_rollback() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"the previous binary").unwrap();
	let backup = move_target_aside(&target).expect("the move-aside must succeed");
	assert!(
		backup.exists(),
		"the .old sibling must exist after the swap window, for the self-test to roll back to"
	);
	assert_eq!(
		std::fs::read(&backup).unwrap(),
		b"the previous binary",
		"the .old must hold the bytes that were at the target before the swap"
	);
	#[cfg(not(windows))]
	{
		assert!(
			target.exists(),
			"on Unix the target stays in place so write_temp can read its mode (`#1898`)"
		);
		assert_eq!(
			std::fs::read(&target).unwrap(),
			b"the previous binary",
			"the target must still hold the pre-swap bytes on Unix"
		);
	}
	#[cfg(windows)]
	assert!(
		!target.exists(),
		"on Windows the running .exe must be moved aside, not left in place"
	);
}

/// #1360 (L5): once the self-test passes, the `.old` sibling is no longer
/// needed and is reaped. `install_binary` does this directly, but the
/// behaviour here is the assertion that the helper functions are composed
/// correctly: `move_target_aside` + `restore_from_backup` round-trips the
/// original bytes through the `.old` path.
#[test]
fn restore_from_backup_round_trips_the_old_binary() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"the previous binary").unwrap();
	let backup = move_target_aside(&target).expect("the move-aside must succeed");
	// Pretend the swap installed a new binary. The rollback path is now
	// the user's safety net.
	// The swap is a rename in production, never an in-place write.
	install_at(&target, b"the new binary (failing self-test)").unwrap();
	restore_from_backup(&target, &backup).expect("the rollback must succeed");
	assert_eq!(
		std::fs::read(&target).unwrap(),
		b"the previous binary",
		"the rollback must put the previous binary back at the target"
	);
	assert!(
		!backup.exists(),
		"the .old sibling is consumed by the rollback"
	);
}

/// No previous file means no backup; a caller requiring rollback must fail.
#[test]
fn move_target_aside_is_a_no_op_when_the_target_does_not_exist() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	// No target to begin with, so the move-aside must still return a
	// backup path (the caller always has a place to roll back to / from).
	let backup = move_target_aside(&target).expect("the move-aside must succeed");
	assert!(
		!backup.exists(),
		"no .old is created when there was no target"
	);
	assert!(!target.exists(), "a non-existent target stays non-existent");
	assert!(restore_from_backup(&target, &backup).is_err());
}

#[test]
fn a_missing_backup_does_not_claim_to_restore_the_previous_binary() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"failed update").unwrap();
	let err = restore_from_backup(&target, &target.with_extension("old")).unwrap_err();
	assert!(err.to_string().contains("roll back"), "{err}");
	assert_eq!(std::fs::read(&target).unwrap(), b"failed update");
}

/// The Unix backup is a hard link of the original: the inode survives at
/// `.old` until the swap renames the staged file over `target`. Reading the
/// previous bytes from `.old` after the swap is what the rollback relies on
/// in `install_binary`, and it depends on the inode, not a copy of it.
#[cfg(unix)]
#[test]
fn the_backup_is_the_original_inode_until_the_swap() {
	use std::os::unix::fs::MetadataExt;
	let dir = tempfile::tempdir().expect("tempdir");
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").expect("write target");
	let before = std::fs::metadata(&target).expect("metadata target").ino();
	let backup = move_target_aside(&target).expect("the move-aside must succeed");
	assert_eq!(
		std::fs::metadata(&backup).expect("metadata backup").ino(),
		before,
		"the backup must hold the original inode on disk"
	);
	assert_eq!(
		std::fs::metadata(&target).expect("metadata target").ino(),
		before,
		"the target and the backup are the same inode until the swap"
	);
	install_at(&target, b"new").expect("the install must succeed");
	assert_eq!(
		std::fs::metadata(&backup).expect("metadata backup").ino(),
		before,
		"the original file must still be reachable at .old after the swap"
	);
	assert_ne!(
		std::fs::metadata(&target).expect("metadata target").ino(),
		before,
		"the staged file must have replaced the target's inode"
	);
	assert_eq!(
		std::fs::read(&backup).expect("read backup"),
		b"old",
		"the rollback must be able to read the original bytes"
	);
	assert_eq!(
		std::fs::read(&target).expect("read target"),
		b"new",
		"the swap must put the new bytes at the target"
	);
}

/// A mode-0111 binary (executable, not readable) can still self-update:
/// `hard_link(2)` only needs execute permission, `rename(2)` only needs
/// write permission on the directory, and the staged swap renames the new
/// file into the target's place. `fs::copy` would have failed because it
/// needs to read the source. Root bypasses the read check, so the test
/// skips itself there: it would prove nothing on an EUID of 0.
#[cfg(unix)]
#[test]
#[allow(unsafe_code)]
fn an_execute_only_binary_can_still_be_updated() {
	use std::os::unix::fs::PermissionsExt;
	// SAFETY: geteuid takes no arguments and cannot fail.
	if unsafe { libc::geteuid() } == 0 {
		eprintln!("skipped: root can read a 0111 file");
		return;
	}
	let dir = tempfile::tempdir().expect("tempdir");
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").expect("write target");
	std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o111)).expect("set mode");
	let backup = move_target_aside(&target).expect("the move-aside must succeed on a 0111 binary");
	install_at(&target, b"new").expect("the install must succeed on a 0111 binary");
	assert_eq!(
		std::fs::metadata(&target)
			.expect("metadata target")
			.permissions()
			.mode() & 0o7777,
		0o111,
		"the operator's mode must survive the update"
	);
	// Restore readable modes so the test can compare bytes; rolling back
	// the *real* swap would not change them, but a future chmod-safety
	// test would.
	std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o644)).expect("backup mode");
	std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).expect("target mode");
	assert_eq!(
		std::fs::read(&target).expect("read target"),
		b"new",
		"the swap must put the new bytes at the target"
	);
	assert_eq!(
		std::fs::read(&backup).expect("read backup"),
		b"old",
		"the .old must still hold the pre-swap bytes"
	);
}

/// If staging fails before `swap_into_place`, `install_binary` calls
/// `restore_from_backup`. When the swap never happened, target and backup
/// still point at the same inode (the hard link), so renaming the backup
/// onto the target would be a no-op and leave `.old` behind. The rollback
/// path detects that and drops the backup instead, leaving the original
/// file in place.
#[cfg(unix)]
#[test]
fn a_failure_before_the_swap_leaves_the_original_file_in_place() {
	use std::os::unix::fs::MetadataExt;
	let dir = tempfile::tempdir().expect("tempdir");
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").expect("write target");
	let before = std::fs::metadata(&target).expect("metadata target").ino();
	let backup = move_target_aside(&target).expect("the move-aside must succeed");
	// No `install_at` between the link and the rollback - mimics the
	// `install_binary` failure path when `install_at` errors out.
	restore_from_backup(&target, &backup).expect("the rollback must succeed");
	assert_eq!(
		std::fs::metadata(&target).expect("metadata target").ino(),
		before,
		"the original inode must still be at the target"
	);
	assert_eq!(
		std::fs::read(&target).expect("read target"),
		b"old",
		"the target must still hold the original bytes"
	);
	assert!(!backup.exists(), "the unused backup must be reaped");
}

#[cfg(windows)]
#[test]
fn cleanup_stale_backup_removes_a_leftover_old_file() {
	// Simulates the case swap_into_place leaves behind: an `.old` sibling of
	// the running executable that its own best-effort delete could not
	// remove because the old process still held it open. The next updater
	// run calls this once nothing holds the file anymore, and it must go.
	let exe = std::env::current_exe().unwrap();
	let backup = exe.with_extension("old");
	std::fs::write(&backup, b"leftover backup").unwrap();
	cleanup_stale_backup();
	assert!(!backup.exists(), "the stale .old backup must be removed");
}

#[cfg(windows)]
#[test]
fn cleanup_stale_backup_is_a_no_op_without_a_leftover() {
	// No `.old` file present is the common case (a normal run, or a
	// platform that never took the Windows swap path) - must not error.
	let exe = std::env::current_exe().unwrap();
	let backup = exe.with_extension("old");
	let _ = std::fs::remove_file(&backup);
	cleanup_stale_backup();
	assert!(!backup.exists());
}
