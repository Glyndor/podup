use super::*;

/// A packing error midway reaches the caller as a `ComposeError`, not as a
/// truncated upload whose post-PUT confirmation then reports a generic
/// "could not be confirmed". The original cause — a file the packer could
/// not read — is the more useful answer, and it must survive the body stream
/// ending early. Without this guard, a permission-denied or vanished file
/// would look like a clean upload that just happened to lose the connection.
#[cfg(unix)]
#[tokio::test]
async fn a_pack_error_midway_reaches_the_caller_as_an_error() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(&payload).unwrap();
	std::fs::write(payload.join("ok.txt"), b"ok").unwrap();
	let blocked = payload.join("blocked");
	std::fs::create_dir(&blocked).unwrap();
	std::fs::write(blocked.join("secret"), b"hidden").unwrap();
	let perm_ro = std::fs::Permissions::from_mode(0o000);
	std::fs::set_permissions(&blocked, perm_ro).unwrap();
	// Confirm the gate is closed; bail out if the test environment can't
	// enforce it (running as root, for example).
	if std::fs::read_dir(&blocked).is_ok() {
		let _ = std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o755));
		return;
	}

	// The runtime hangs up after the PUT (Podman 6 / #1097): only the
	// apply-then-close path forces the post-PUT confirmation to run, which
	// is the branch that surfaces the producer's error. A clean 200
	// (Podman 5) short-circuits to `Ok` before the producer runs, so the
	// caller never sees the pack-time cause.
	let fake = runtime(Put::HangsUp, &[("/tmp", OnDisk::Dir)]);
	let result = upload(&fake, &payload, "payload", None).await;
	let _ = std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o755));

	// The packer errors with `ComposeError::Build("cp: ... permission
	// denied")` or `ComposeError::Io(PermissionDenied)` before the PUT
	// finishes; the user sees the original cause, not the generic "could
	// not be confirmed" the post-PUT verification would produce on a
	// truncated body.
	let msg = match result {
		Err(ComposeError::Build(msg)) => msg,
		Err(ComposeError::Io(err)) => err.to_string(),
		other => panic!(
			"a mid-pack error must surface as an error carrying the original cause, got {other:?}"
		),
	};
	assert!(
		msg.contains("cp:")
			|| msg.contains("permission")
			|| msg.contains("denied")
			|| msg.contains("Permission"),
		"the message must carry the original cause, got: {msg}"
	);
}
