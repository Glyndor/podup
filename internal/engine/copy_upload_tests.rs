//! What the archive upload reports when the runtime hangs up without answering.
//!
//! Podman 6 applies an archive PUT and closes the connection with no response
//! (#1097), so the upload is confirmed by reading the destination back. These
//! drive that whole exchange against the fake socket: the PUT is dropped the way
//! Podman 6 drops it, and the stat `HEAD`s are answered from a table standing in
//! for the container's filesystem. What is asserted is the caller's view, `Ok`
//! or which error, for a payload that arrived and for one that did not.
//!
//! The stat header is built in the shape Podman 5.7.0 sent on 2026-09-18:
//! `{"name":"a.txt","size":6,"mode":420,"mtime":"…","isDir":false,"linkTarget":"/tmp/t/a.txt"}`
//! for a file and `"mode":2147484141` for a directory.

use std::path::Path;

use base64::Engine as _;

use super::{pack_path, uploaded_entry_kind};
use crate::engine::fake_podman::{self, FakePodman, FakeReply};
use crate::engine::Engine;
use crate::error::ComposeError;
use crate::libpod::urlencoded;

const CONTAINER: &str = "proj-web-1";

/// One path in the fake container.
#[derive(Clone, Copy)]
enum OnDisk {
	File(u64),
	Dir,
	/// A symbolic link; the stat header carries its size and the
	/// `os.ModeSymlink` mode bit (1<<27 | 0o777).
	Link(u64),
	/// A named pipe; the stat header carries size 0 and the
	/// `os.ModeNamedPipe` mode bit (1<<25 | 0o644).
	Fifo,
	/// There, but the runtime fails the stat with a 500.
	Unreadable,
}

/// How the fake answers the archive PUT.
#[derive(Clone, Copy)]
enum Put {
	/// Accept the body and close without a response: Podman 6.
	HangsUp,
	/// A normal response with this status: Podman 5.
	Answers(u16),
}

fn stat_header(path: &str, entry: OnDisk) -> String {
	let name = path.rsplit('/').next().unwrap_or_default();
	let (size, mode, is_dir) = match entry {
		OnDisk::File(size) => (size, 420u64, false),
		OnDisk::Dir => (4096, 2_147_484_141, true),
		OnDisk::Link(size) => (size, (1u64 << 27) | 0o777, false),
		OnDisk::Fifo => (0, (1u64 << 25) | 0o644, false),
		OnDisk::Unreadable => unreachable!("answered with a 500, not a stat"),
	};
	let json = format!(
		r#"{{"name":"{name}","size":{size},"mode":{mode},"mtime":"2026-09-18T19:50:59.194580835-05:00","isDir":{is_dir},"linkTarget":"{path}"}}"#
	);
	base64::engine::general_purpose::STANDARD.encode(json)
}

/// A runtime that treats the PUT as `put` says and whose container holds
/// exactly `disk`. Nothing the PUT carries changes `disk`: whether the upload
/// "landed" is decided by the table the test passes in.
///
/// `link_stat_on_404` controls how the fake answers a `HEAD` for a `Link`
/// entry: Podman 5.7.0 returns 404 with the stat header still on it (the
/// default for `tree_landed`'s link confirmation); a runtime that does not
/// (older, or a stub) drops the header and the link cannot be confirmed.
fn runtime_full(put: Put, disk: &[(&str, OnDisk)], link_stat_on_404: bool) -> FakePodman {
	let disk: Vec<(String, OnDisk)> = disk.iter().map(|(p, e)| ((*p).to_string(), *e)).collect();
	fake_podman::start_replying(move |method, target| match method {
		"PUT" => match put {
			Put::HangsUp => FakeReply::ClosedWithoutResponse,
			Put::Answers(status) => FakeReply::Body(status, r#"{"message":"refused"}"#.into()),
		},
		"HEAD" => disk
			.iter()
			.find(|(path, _)| target.ends_with(&format!("archive?path={}", urlencoded(path))))
			.map(|(path, entry)| match entry {
				OnDisk::Unreadable => FakeReply::Headers(500, Vec::new()),
				OnDisk::Link(_) if link_stat_on_404 => FakeReply::Headers(
					404,
					vec![("X-Docker-Container-Path-Stat", stat_header(path, *entry))],
				),
				_ => FakeReply::Headers(
					200,
					vec![("X-Docker-Container-Path-Stat", stat_header(path, *entry))],
				),
			})
			.unwrap_or(FakeReply::Headers(404, Vec::new())),
		_ => FakeReply::Body(404, r#"{"message":"not found"}"#.into()),
	})
}

/// As [`runtime_full`], with the Podman 5.7.0 link-stat-on-404 behaviour
/// (the default for every pre-existing test, which carries no links).
fn runtime(put: Put, disk: &[(&str, OnDisk)]) -> FakePodman {
	runtime_full(put, disk, true)
}

fn engine_for(fake: &FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

/// `payload/` with a file at the top, a file one level down and an empty
/// directory, which between them are every kind of entry a tree upload has to
/// account for.
fn payload_tree(root: &Path) -> std::path::PathBuf {
	let payload = root.join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::create_dir(payload.join("empty")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"beside-the-rest").unwrap();
	std::fs::write(payload.join("sub/inner.bin"), vec![7u8; 1234]).unwrap();
	payload
}

/// The container after `payload_tree` was extracted at `/tmp`.
const LANDED_TREE: [(&str, OnDisk); 5] = [
	("/tmp/payload", OnDisk::Dir),
	("/tmp/payload/plain.txt", OnDisk::File(15)),
	("/tmp/payload/sub", OnDisk::Dir),
	("/tmp/payload/sub/inner.bin", OnDisk::File(1234)),
	("/tmp/payload/empty", OnDisk::Dir),
];

/// Upload `src` to `/tmp` under `entry`, exactly as `cp` does it.
async fn upload(
	fake: &FakePodman,
	src: &Path,
	entry: &str,
	rename: Option<&str>,
) -> crate::error::Result<()> {
	let tar = pack_path(src, false, rename, false).unwrap();
	engine_for(fake)
		.put_archive_verified(
			CONTAINER,
			"/tmp",
			entry,
			tar,
			uploaded_entry_kind(src, false),
			super::CpByteCounter::new(),
		)
		.await
}

fn assert_unconfirmed(result: crate::error::Result<()>, case: &str) {
	match result {
		Err(ComposeError::Copy(msg)) => assert!(
			msg.contains("could not be confirmed"),
			"{case}: refused, but not as an unconfirmed upload: {msg}"
		),
		other => panic!("{case}: must be reported as an unconfirmed upload, got {other:?}"),
	}
}

/// Extract the inner detail the caller wraps, for the assertion that the
/// user-facing error names the entry and the stat. Today the message reaches
/// the user via `ComposeError::Copy(_)`; this is the seam the tests read.
fn copy_detail(result: crate::error::Result<()>, case: &str) -> String {
	match result {
		Err(ComposeError::Copy(msg)) => msg,
		other => panic!("{case}: must be reported as an unconfirmed copy, got {other:?}"),
	}
}

/// #1777. The copy landed, the runtime hung up, and the caller was told it
/// failed, because a directory gave the confirmation nothing to compare.
#[tokio::test]
async fn a_directory_that_landed_is_confirmed_when_the_runtime_hangs_up() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let fake = runtime(Put::HangsUp, &LANDED_TREE);

	let result = upload(&fake, &payload, "payload", None).await;

	assert!(
		result.is_ok(),
		"every entry of the tree is at the destination, got {result:?}"
	);
}

/// The other half, and the one that matters more: hanging up is not evidence.
/// Nothing of the payload is in the container, so nothing may be confirmed.
#[tokio::test]
async fn a_directory_that_did_not_land_is_still_a_failure() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let fake = runtime(Put::HangsUp, &[("/tmp", OnDisk::Dir)]);

	let result = upload(&fake, &payload, "payload", None).await;

	assert_unconfirmed(result, "empty destination");
}

/// A stream cut part of the way leaves some of the tree behind. Take each entry
/// away in turn, with whatever lived under it, and the upload must be refused
/// every time: confirming on the top directory, or on any one child, would
/// pass some of these.
#[tokio::test]
async fn a_directory_that_landed_in_part_is_a_failure_whichever_part_is_missing() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());

	for (missing, _) in LANDED_TREE {
		let partial: Vec<(&str, OnDisk)> = LANDED_TREE
			.into_iter()
			.filter(|(path, _)| *path != missing && !path.starts_with(&format!("{missing}/")))
			.collect();
		let fake = runtime(Put::HangsUp, &partial);

		let result = upload(&fake, &payload, "payload", None).await;

		assert_unconfirmed(result, &format!("without {missing}"));
	}
}

/// Present is not the same as arrived: an entry left over from before the
/// upload, at another size or of another kind, is not what was sent.
#[tokio::test]
async fn a_directory_whose_entries_do_not_match_what_was_sent_is_a_failure() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let stale: [(&str, OnDisk, &str); 3] = [
		(
			"/tmp/payload/plain.txt",
			OnDisk::File(14),
			"a file one byte short",
		),
		(
			"/tmp/payload/sub/inner.bin",
			OnDisk::File(0),
			"a file that is empty",
		),
		(
			"/tmp/payload/empty",
			OnDisk::File(0),
			"a file where a directory was sent",
		),
	];

	for (path, instead, case) in stale {
		let disk: Vec<(&str, OnDisk)> = LANDED_TREE
			.into_iter()
			.map(|(p, e)| if p == path { (p, instead) } else { (p, e) })
			.collect();
		let fake = runtime(Put::HangsUp, &disk);

		let result = upload(&fake, &payload, "payload", None).await;

		assert_unconfirmed(result, case);
	}
}

/// The error message the caller sees names the entry that failed AND the
/// stat the runtime answered, not just the verdict. This is what makes the
/// next diagnosis not blind: a log carries the path and the literal
/// `PathStat`, and an operator can act on it without re-running the upload.
#[tokio::test]
async fn a_refused_entry_message_names_the_path_and_the_stat() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());

	// One entry is stale: `payload/plain.txt` was meant to be 15 bytes and
	// arrived at 14. The path is what the user can look at; the stat is the
	// evidence the runtime said it was 14 bytes.
	let stale = "/tmp/payload/plain.txt";
	let disk: Vec<(&str, OnDisk)> = LANDED_TREE
		.into_iter()
		.map(|(p, e)| {
			if p == stale {
				(p, OnDisk::File(14))
			} else {
				(p, e)
			}
		})
		.collect();
	let fake = runtime(Put::HangsUp, &disk);

	let result = upload(&fake, &payload, "payload", None).await;
	let msg = copy_detail(result, "stale plain.txt");

	// The path of the entry that failed, relative to the extract dir, is in
	// the message. An operator looking at the log can see this entry.
	assert!(
		msg.contains("payload/plain.txt"),
		"the entry path must be named in the message: {msg}"
	);

	// The literal `PathStat` from the runtime is in the message. The Debug
	// rendering of `PathStat` carries the size and mode the runtime reported,
	// which is what the next diagnosis needs to compare expected and answered.
	assert!(
		msg.contains("size"),
		"the literal PathStat (whose Debug starts with `size`) must be in the \
		 message: {msg}"
	);
	assert!(
		msg.contains("14"),
		"the runtime's reported size (14) must be in the message: {msg}"
	);

	// The expected kind is also named, so the next diagnosis knows what the
	// upload was supposed to land.
	assert!(
		msg.contains("File(15)"),
		"the expected kind (a 15-byte file) must be in the message: {msg}"
	);

	// The actionable hint the existing message carried stays: it is true
	// (the bytes may have landed) and useful (the operator can look).
	assert!(
		msg.contains("may or may not have landed"),
		"the existing hint must stay: {msg}"
	);
}

/// A stat the runtime would not answer is not an answer. Everything else of the
/// tree is in place, so only the unreadable entry can be what refuses it.
#[tokio::test]
async fn a_directory_with_an_entry_that_cannot_be_read_back_is_a_failure() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());

	for (unreadable, _) in LANDED_TREE {
		let disk: Vec<(&str, OnDisk)> = LANDED_TREE
			.into_iter()
			.map(|(p, e)| {
				(
					p,
					if p == unreadable {
						OnDisk::Unreadable
					} else {
						e
					},
				)
			})
			.collect();
		let fake = runtime(Put::HangsUp, &disk);

		let result = upload(&fake, &payload, "payload", None).await;

		assert_unconfirmed(result, &format!("{unreadable} answers 500"));
	}
}

/// A dangling symlink is packed as a link, and a link is nothing the stat
/// endpoint can be asked about. With nothing to ask, nothing is confirmed:
/// an upload is never a success for want of a question.
#[tokio::test]
async fn an_upload_with_nothing_to_ask_about_stays_unconfirmed() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("dangling");
	std::os::unix::fs::symlink("nowhere", &link).unwrap();
	let fake = runtime(Put::HangsUp, &[("/tmp", OnDisk::Dir)]);

	let result = upload(&fake, &link, "dangling", None).await;

	assert_unconfirmed(result, "a lone symlink");
}

/// `cp dir svc:/tmp/renamed` packs the tree under the new name, and that name
/// is where it has to be looked for.
#[tokio::test]
async fn a_renamed_directory_is_confirmed_under_its_new_name() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let renamed: Vec<(String, OnDisk)> = LANDED_TREE
		.into_iter()
		.map(|(p, e)| (p.replacen("/tmp/payload", "/tmp/renamed", 1), e))
		.collect();
	let renamed: Vec<(&str, OnDisk)> = renamed.iter().map(|(p, e)| (p.as_str(), *e)).collect();

	let landed = runtime(Put::HangsUp, &renamed);
	let result = upload(&landed, &payload, "renamed", Some("renamed")).await;
	assert!(
		result.is_ok(),
		"the tree is there as `renamed`, got {result:?}"
	);

	// Under the source's own name it is somebody else's tree.
	let elsewhere = runtime(Put::HangsUp, &LANDED_TREE);
	let result = upload(&elsewhere, &payload, "renamed", Some("renamed")).await;
	assert_unconfirmed(result, "only the old name exists");
}

/// The single-file answers, held where they were: confirmed on its size, and
/// refused when the entry is absent or is still the old one.
#[tokio::test]
async fn a_single_file_is_confirmed_on_its_size_and_on_nothing_less() {
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("f.txt");
	std::fs::write(&file, b"fifteen bytes!!").unwrap();

	let landed = runtime(Put::HangsUp, &[("/tmp/f.txt", OnDisk::File(15))]);
	let result = upload(&landed, &file, "f.txt", None).await;
	assert!(
		result.is_ok(),
		"the file is there at its size, got {result:?}"
	);

	let absent = runtime(Put::HangsUp, &[("/tmp", OnDisk::Dir)]);
	assert_unconfirmed(upload(&absent, &file, "f.txt", None).await, "absent file");

	let old = runtime(Put::HangsUp, &[("/tmp/f.txt", OnDisk::File(14))]);
	assert_unconfirmed(
		upload(&old, &file, "f.txt", None).await,
		"the previous file",
	);

	let a_directory = runtime(Put::HangsUp, &[("/tmp/f.txt", OnDisk::Dir)]);
	assert_unconfirmed(
		upload(&a_directory, &file, "f.txt", None).await,
		"a directory of that name",
	);
}

/// Only the hang-up is recoverable. A runtime that answered with an error said
/// what happened, and a destination that looks right does not overrule it.
#[tokio::test]
async fn an_answered_refusal_is_not_recovered_by_a_matching_destination() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let fake = runtime(Put::Answers(500), &LANDED_TREE);

	let result = upload(&fake, &payload, "payload", None).await;

	assert!(
		matches!(result, Err(ComposeError::Podman(_))),
		"the runtime's own error must reach the caller, got {result:?}"
	);
}

/// Podman 5 answers the PUT, and that answer is the confirmation: nothing is
/// read back, so a tree costs it no extra requests.
#[tokio::test]
async fn an_answered_upload_is_not_read_back() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let fake = runtime(Put::Answers(200), &[]);

	let result = upload(&fake, &payload, "payload", None).await;

	assert!(result.is_ok(), "a 200 is success, got {result:?}");
	let requests = fake.requests.lock().unwrap().clone();
	assert_eq!(
		requests.len(),
		1,
		"one PUT and nothing else, got {requests:?}"
	);
	assert!(requests[0].starts_with("PUT "), "got {requests:?}");
}

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
		("/tmp/payload/link", OnDisk::Link(4)),
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
		("/tmp/payload/link", OnDisk::Link(9)),
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

	let landed: [(&str, OnDisk); 1] = [("/tmp/dangling", OnDisk::Link(7))];
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
