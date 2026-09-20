//! Link-target confirmation through the fake Podman.
//!
//! These tests cover the link-target half of the confirmation against the
//! fake socket: PUT dropped the Podman 6 way, the link's stat `HEAD`
//! answered with whatever the table says, and the upload is either reported
//! as landed or as unconfirmed accordingly. Split out from
//! `copy_upload_tests.rs` so the file does not cross the 500 code-line hard
//! limit; the helpers they need (`OnDisk`, `Put`, `runtime_full`, `upload`,
//! `assert_unconfirmed`, `CONTAINER`) are exposed as `pub(super)` for this
//! reason.

use super::upload_tests::{assert_unconfirmed, runtime_full, upload, OnDisk, Put};

/// Podman 6 has been seen answering a link's stat `HEAD` with a 404 that
/// carries the symlink stat but omits `linkTarget`. That is the shape that
/// re-introduced #1777 for directory copies against Podman 6 once the link
/// confirmation went in. The fake reproduces the omission, the PUT is
/// dropped the Podman 6 way, and the upload must be reported as landed: the
/// runtime answered a symlink, the destination cannot be asked any stronger
/// than the symlink bit, and refusing a copy that landed would be the wrong
/// answer. The call site emits a `tracing::warn!` carrying the literal stat;
/// the regression net here is the `Ok` that lands.
#[cfg(unix)]
#[tokio::test]
async fn a_directory_with_a_link_is_confirmed_when_the_runtime_omits_link_target() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::os::unix::fs::symlink("a.txt", payload.join("link")).unwrap();

	let disk: [(&str, OnDisk); 2] = [
		("/tmp/payload", OnDisk::Dir),
		("/tmp/payload/link", OnDisk::LinkNoTarget { size: 4 }),
	];
	let fake = runtime_full(Put::HangsUp, &disk, true);

	let result = upload(&fake, &payload, "payload", None).await;

	assert!(
		result.is_ok(),
		"the destination is a symlink; the runtime did not report linkTarget; \
		 the copy must still land, got {result:?}"
	);
}

/// Podman 6 has also been seen answering a link's stat `HEAD` with the
/// `linkTarget` field present but set to the empty string. A symlink always
/// points at something, so an empty string is not a valid symlink target and
/// a runtime reporting `""` has not answered the question, exactly as one
/// that omits the field has not. The fake reproduces that shape, the PUT is
/// dropped the Podman 6 way, and the upload must still be reported as
/// landed: the previous behaviour (treating `""` as a target to compare
/// against, refusing the entry, and reporting a copy that landed as failed)
/// is the #1777 shape this module exists to close. The call site emits a
/// `tracing::warn!` carrying the literal stat; the regression net here is the
/// `Ok` that lands. Beside the omitted-field case above.
#[cfg(unix)]
#[tokio::test]
async fn a_directory_with_a_link_is_confirmed_when_the_runtime_reports_an_empty_link_target() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::os::unix::fs::symlink("a.txt", payload.join("link")).unwrap();

	let disk: [(&str, OnDisk); 2] = [
		("/tmp/payload", OnDisk::Dir),
		("/tmp/payload/link", OnDisk::LinkEmptyTarget { size: 4 }),
	];
	let fake = runtime_full(Put::HangsUp, &disk, true);

	let result = upload(&fake, &payload, "payload", None).await;

	assert!(
		result.is_ok(),
		"the destination is a symlink; the runtime reported linkTarget as the empty string; \
		 the copy must still land, the same as the omitted-field case, got {result:?}"
	);
}

/// The same archive, on a runtime whose `linkTarget` for the link does NOT
/// equal the target the archive carried. The copy did not land as the
/// archive described it (the destination link points somewhere else), so
/// confirming on the symlink bit alone would call it landed and leave a
/// wrong link at the destination. The target check rejects it, the upload
/// is reported as failed. This is the regression net for the strong path:
/// the no-target fallback above holds only because the runtime did not
/// answer the field at all, not because the answer disagreed.
#[cfg(unix)]
#[tokio::test]
async fn a_directory_with_a_link_is_a_failure_when_the_runtime_target_differs_from_the_archive() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::os::unix::fs::symlink("a.txt", payload.join("link")).unwrap();

	let disk: [(&str, OnDisk); 2] = [
		("/tmp/payload", OnDisk::Dir),
		(
			"/tmp/payload/link",
			OnDisk::Link {
				size: 9,
				target: "somewhere-else",
			},
		),
	];
	let fake = runtime_full(Put::HangsUp, &disk, true);

	let result = upload(&fake, &payload, "payload", None).await;

	assert_unconfirmed(
		result,
		"a destination link whose target differs from the archive must not be confirmed",
	);
}
