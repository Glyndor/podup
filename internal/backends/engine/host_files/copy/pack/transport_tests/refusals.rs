use super::*;

// --- what the packer refuses ----------------------------------------------

/// A FIFO on the source makes the packer refuse up front, the same way
/// `sent_entries` reading the archive back would refuse. Closing the gate at
/// pack time means the body stream never gets a half-built archive, and the
/// confirmation never sees an entry it cannot ask about.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn a_fifo_source_is_refused_by_the_streaming_packer() {
	let dir = tempfile::tempdir().unwrap();
	let pipe = dir.path().join("pipe");
	// `mkfifo` is not in std; the FIFO is created with the right mode bits via
	// `mknod`. Skip on systems that do not expose it.
	let status = std::process::Command::new("mknod")
		.args([pipe.to_str().unwrap(), "p"])
		.status();
	let Ok(status) = status else { return };
	if !status.success() {
		return;
	}

	let (_recorded, result) = run(pack_path_stream(&pipe, false, None, false, counter())).await;
	let err = result.expect_err("a FIFO source must make the packer error");
	let msg = err.to_string();
	assert!(
		msg.contains("unverified kind") || msg.contains("Fifo"),
		"the error must name the unverifiable kind, got: {msg}"
	);
}

/// A packing error midway reaches the caller as a `ComposeError`. Without
/// this, a permission denied or vanished file would surface only as a
/// truncated upload whose post-PUT confirmation then reports "did not land",
/// masking the original cause.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn a_pack_error_midway_surfaces_as_an_error() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(&payload).unwrap();
	std::fs::write(payload.join("ok.txt"), b"ok").unwrap();

	// Make the second entry unreadable mid-walk: a directory whose contents
	// cannot be enumerated (`chmod 000` on a directory reads as "permission
	// denied" through `walk_dir`'s `read_dir`). The packer reads the file
	// first (so the entry shows up), then the directory (`pack_walk` emits
	// the directory entry, then descends into children that fail).
	let blocked = payload.join("blocked");
	std::fs::create_dir(&blocked).unwrap();
	std::fs::write(blocked.join("secret"), b"hidden").unwrap();
	let perm_ro = std::fs::Permissions::from_mode(0o000);
	std::fs::set_permissions(&blocked, perm_ro).unwrap();
	// Try to read inside, to be sure the gate is closed.
	let probe = std::fs::read_dir(&blocked);
	if probe.is_ok() {
		// Running as root or in a mode where the gate does not hold: the test
		// is not meaningful here. Restore the permissions and skip.
		let _ = std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o755));
		return;
	}

	let (_recorded, result) = run(pack_path_stream(&payload, false, None, false, counter())).await;
	let _ = std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o755));

	let err = result.expect_err("an unreadable entry must make the packer error");
	let msg = err.to_string();
	assert!(
		msg.contains("cp:") || msg.contains("permission") || msg.contains("denied"),
		"the error must surface as a cp / permission / denied message, got: {msg}"
	);
}

/// The path used by `record_one` for the link target is what the destination
/// stat is asked to match. A non-UTF-8 target would survive into the tar but
/// fail to compare against `stat.link_target`. The gate fires at pack time,
/// not at confirm time, so the caller sees the original cause.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn a_symlink_with_a_non_utf_8_target_is_refused() {
	use std::os::unix::ffi::OsStrExt;
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("link");
	std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(b"bad-\xff-target"), &link).unwrap();

	let (_recorded, result) = run(pack_path_stream(&link, false, None, false, counter())).await;
	let err = result.expect_err("a non-UTF-8 link target must make the packer error");
	let msg = err.to_string();
	assert!(
		msg.contains("UTF-8") || msg.contains("utf"),
		"the error must mention the non-UTF-8 link, got: {msg}"
	);
}
