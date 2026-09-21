//! Upload confirmation for symbolic links and the kinds the stat cannot vouch
//! for (FIFOs), driven against the same fake socket and stat table as the
//! parent module.

use std::path::Path;

use super::*;

/// A directory containing only a symbolic link. On Podman 5.7.0 the link
/// answers the stat `HEAD` with 404 that still carries the link's stat header;
/// the runtime reads the link back and the upload is confirmed.
#[cfg(unix)]
#[tokio::test]
async fn a_directory_with_a_link_is_confirmed_when_the_404_carries_the_stat() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::os::unix::fs::symlink("a.txt", payload.join("link")).unwrap();

	let disk: [(&str, OnDisk); 2] = [
		("/tmp/payload", OnDisk::Dir),
		(
			"/tmp/payload/link",
			OnDisk::Link {
				size: 5,
				target: "a.txt".to_string(),
			},
		),
	];
	let fake = runtime_full(Put::HangsUp, &disk, true);

	let result = upload(&fake, &payload, "payload", None).await;

	assert!(
		result.is_ok(),
		"the directory and its link are both at the destination, got {result:?}"
	);
}

/// The same archive, on a runtime whose 404 for the link does NOT carry the
/// stat header. The link cannot be asked about, the upload is not confirmed,
/// and the caller is told so. This is the cut-stream shape the link code was
/// written to refuse.
#[cfg(unix)]
#[tokio::test]
async fn a_directory_with_a_link_is_a_failure_when_the_404_omits_the_stat() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::os::unix::fs::symlink("a.txt", payload.join("link")).unwrap();

	let disk: [(&str, OnDisk); 1] = [("/tmp/payload", OnDisk::Dir)];
	let fake = runtime_full(Put::HangsUp, &disk, false);

	let result = upload(&fake, &payload, "payload", None).await;

	assert_unconfirmed(result, "the link's 404 is missing the stat header");
}

/// An empty file uploaded over an existing named pipe at the destination is
/// NOT confirmed by the unchanged pipe: the regular-file check rejects any
/// mode with a `ModeType` bit set, and a FIFO at size 0 would otherwise pass.
#[tokio::test]
async fn an_empty_file_uploaded_over_a_fifo_is_a_failure() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::fs::write(payload.join("plain"), b"").unwrap();

	let disk: [(&str, OnDisk); 2] = [
		("/tmp/payload", OnDisk::Dir),
		("/tmp/payload/plain", OnDisk::Fifo),
	];
	let fake = runtime_full(Put::HangsUp, &disk, true);

	let result = upload(&fake, &payload, "payload", None).await;

	assert_unconfirmed(result, "a FIFO at the destination is not a regular file");
}

/// An archive with a directory and a FIFO is unverifiable: the FIFO cannot
/// be asked about through the archive stat, so even when the runtime answers
/// the directory's stat with 200 the upload cannot be confirmed against the
/// directory alone. Without this guard, the FIFO would have been filtered
/// out by `sent_entries` and the directory alone would have passed.
#[cfg(unix)]
#[tokio::test]
async fn a_directory_with_a_fifo_is_a_failure_even_when_the_dir_lands() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	// Plant the FIFO via `mkfifo`; a plain `std::fs::File::create` would
	// not yield a FIFO that the packer would emit, which is the case the
	// regression net has to cover.
	mkfifo(&payload.join("pipe"));

	let disk: [(&str, OnDisk); 2] = [
		("/tmp/payload", OnDisk::Dir),
		("/tmp/payload/pipe", OnDisk::Fifo),
	];
	let fake = runtime_full(Put::HangsUp, &disk, true);

	let result = upload(&fake, &payload, "payload", None).await;

	assert_unconfirmed(
		result,
		"a FIFO alongside a directory is unverifiable, so the upload must fail closed",
	);
}

/// Plant a named pipe at `path`. `std::fs` has no FIFO constructor, and `nix`
/// is not a dependency, so this one-line `libc::mkfifo` call is the only
/// way to put a FIFO on disk from a test. The `unsafe` is bounded to this
/// helper so the rest of the file keeps the crate-wide `deny(unsafe_code)`.
#[cfg(unix)]
#[allow(unsafe_code)]
fn mkfifo(path: &Path) {
	use std::ffi::CString;
	use std::os::unix::ffi::OsStrExt;
	let c_path = CString::new(path.as_os_str().as_bytes()).expect("cstring");
	let rc = unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) };
	assert_eq!(rc, 0, "mkfifo failed: {}", std::io::Error::last_os_error());
}

/// An archive holding only the three kinds the destination can be asked about
/// (a file, a directory, a symlink) is still confirmed the same way it was
/// before the FIFO/hard-link filter went in. The regression net for the new
/// `sent_entries` error path: the filter must not have changed which entries
/// come back as `File`, `Dir` or `Link`.
#[cfg(unix)]
#[tokio::test]
async fn an_archive_with_only_the_three_verifiable_kinds_is_confirmed() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir(&payload).unwrap();
	std::fs::create_dir(payload.join("sub")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"hi").unwrap();
	std::os::unix::fs::symlink("plain.txt", payload.join("link")).unwrap();

	let disk: [(&str, OnDisk); 4] = [
		("/tmp/payload", OnDisk::Dir),
		(
			"/tmp/payload/link",
			OnDisk::Link {
				size: 9,
				target: "plain.txt".to_string(),
			},
		),
		("/tmp/payload/plain.txt", OnDisk::File(2)),
		("/tmp/payload/sub", OnDisk::Dir),
	];
	let fake = runtime_full(Put::HangsUp, &disk, true);

	let result = upload(&fake, &payload, "payload", None).await;

	assert!(
		result.is_ok(),
		"every entry of the archive is verifiable and lands, got {result:?}"
	);
}

/// The single-file path closes the same FIFO false positive. An empty file
/// uploaded at a destination that the runtime reports as a zero-length FIFO
/// cannot be confirmed: the previous size-only comparison would have passed
/// on the unchanged pipe, since both report size 0. The mode check rejects
/// the FIFO's `ModeNamedPipe` bit and the upload is refused.
#[tokio::test]
async fn a_single_empty_file_over_a_zero_length_fifo_is_a_failure() {
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("plain");
	std::fs::write(&file, b"").unwrap();

	let landed: [(&str, OnDisk); 1] = [("/tmp/plain", OnDisk::Fifo)];
	let fake = runtime_full(Put::HangsUp, &landed, true);

	let result = upload(&fake, &file, "plain", None).await;

	assert_unconfirmed(
		result,
		"the destination is a FIFO at size 0, not the empty file that was uploaded",
	);
}

/// A symlink source copied without `-L/--follow-link` confirms on the link
/// itself: the archive stores the link, and the destination must answer the
/// stat as a symlink. The 404-with-stat-on-Podman-5.7.0 shape the link code
/// was written for is exercised here, matching how the tree path handles
/// the dangling-link case.
#[cfg(unix)]
#[tokio::test]
async fn a_symlink_source_is_confirmed_when_the_destination_is_a_symlink() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("dangling");
	std::os::unix::fs::symlink("nowhere", &link).unwrap();

	let landed: [(&str, OnDisk); 1] = [(
		"/tmp/dangling",
		OnDisk::Link {
			size: 7,
			target: "nowhere".to_string(),
		},
	)];
	let fake = runtime_full(Put::HangsUp, &landed, true);

	let result = upload(&fake, &link, "dangling", None).await;

	assert!(
		result.is_ok(),
		"the source is a symlink, the destination is a symlink, got {result:?}"
	);
}

/// The same symlink source, but the destination reports itself as a regular
/// file of the same size: the destination is not what was uploaded (the
/// archive stored the link, the destination holds a regular file). The
/// previous size-only comparison would have passed; the kind check rejects
/// the file-type bit.
#[cfg(unix)]
#[tokio::test]
async fn a_symlink_source_is_a_failure_when_the_destination_is_a_regular_file() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("dangling");
	std::os::unix::fs::symlink("nowhere", &link).unwrap();

	let lied: [(&str, OnDisk); 1] = [("/tmp/dangling", OnDisk::File(7))];
	let fake = runtime_full(Put::HangsUp, &lied, true);

	let result = upload(&fake, &link, "dangling", None).await;

	assert_unconfirmed(
		result,
		"a regular file at the destination is not the symlink that was uploaded",
	);
}
