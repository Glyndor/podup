use super::*;

/// A special bit on the target is never propagated onto the new binary.
///
/// `write_temp` copies the target's permissions so an install keeps whatever
/// mode the operator chose, and masks with `& 0o777` on the way. Without the
/// mask, a target that had been made setuid (by tampering, or by an
/// operator who did it on purpose once) would hand the freshly installed
/// podup the same bit, on a binary that has just been fetched over the
/// network. That is a privilege-escalation footgun, and it is the one
/// property of this function a test can actually observe.
///
/// The other three are window guards and cannot be reached in process: the
/// 0600 create mode is overwritten by this very copy before the function
/// returns, and `O_EXCL`/`O_NOFOLLOW` close a race between the unlink above
/// and the open. Their comments say so where they live.
#[cfg(unix)]
#[test]
fn write_temp_never_propagates_a_special_bit_from_the_target() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").unwrap();
	std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o4755)).unwrap();
	// Only meaningful if the filesystem kept the bit; some do not.
	let target_mode = std::fs::metadata(&target).unwrap().permissions().mode();
	if target_mode & 0o4000 == 0 {
		return;
	}
	let tmp = dir.path().join("podup.tmp");
	super::write_temp(&tmp, b"freshly downloaded", &target).unwrap();
	let mode = std::fs::metadata(&tmp).unwrap().permissions().mode();
	assert_eq!(
		mode & 0o7000,
		0,
		"a special bit rode from the target onto the new binary: {mode:o}"
	);
	assert_eq!(
		mode & 0o777,
		0o755,
		"the ordinary permission bits should still be carried over: {mode:o}"
	);
}

#[test]
fn rename_error_calls_out_permission_and_generic_cases() {
	let target = Path::new("/usr/local/bin/podup");
	// A permission error nudges the user toward elevation.
	let perm = rename_error(
		std::io::Error::from(std::io::ErrorKind::PermissionDenied),
		target,
	);
	match perm {
		ComposeError::Update(msg) => {
			assert!(msg.contains("permission denied"));
			assert!(msg.contains("sudo"));
		}
		_ => panic!("expected an Update error"),
	}
	// Any other error reports the underlying failure verbatim.
	let other = rename_error(std::io::Error::other("disk full"), target);
	match other {
		ComposeError::Update(msg) => {
			assert!(msg.contains("failed to install update"));
			assert!(msg.contains("disk full"));
		}
		_ => panic!("expected an Update error"),
	}
}

/// The Windows branch must not let a planted reparse point redirect the
/// verified bytes, which is the invariant `FILE_FLAG_OPEN_REPARSE_POINT`
/// buys. It is NOT that a pre-existing `tmp` is refused: both branches
/// unlink `tmp` first on purpose, so a leftover from an interrupted
/// update does not wedge every later one. An earlier version of this
/// test asserted the refusal and failed on the Windows runner for that
/// reason, which is the useful kind of failure: the assertion and the
/// code disagreed and the code was right.
///
/// Windows-only: the call sits behind `#[cfg(windows)]` and cannot be
/// exercised from a Linux host. `write_temp_never_propagates_a_special_bit_from_the_target`
/// pins the sibling shape for the Unix branch.
#[cfg(windows)]
#[test]
fn write_temp_lands_the_bytes_at_tmp_over_a_leftover_on_windows() {
	let dir = tempfile::tempdir().expect("tempdir");
	let target = dir.path().join("podup");
	std::fs::write(&target, b"old").expect("write target");
	// A leftover from an interrupted update. The contract is that it is
	// replaced, not that it wedges the update.
	let tmp = dir.path().join("podup.update-1234");
	std::fs::write(&tmp, b"planted leftover").expect("write tmp");
	write_temp(&tmp, b"verified bytes", &target).expect("write_temp must replace a leftover tmp");
	assert_eq!(
		std::fs::read(&tmp).expect("read tmp"),
		b"verified bytes",
		"the verified bytes must land at tmp, not the leftover"
	);
}
