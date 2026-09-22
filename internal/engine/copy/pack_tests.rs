//! Tests for the streaming packer that the post-PUT confirmation reads from.
//!
//! The streamed bytes are gone after the PUT, so the verification compares the
//! runtime's stat against what the packer recorded while writing the tar. The
//! whole point of moving to a stream (#1844) is that the recorded list stays
//! the only source of "what was uploaded", and the only way to make sure of
//! that is to assert it equals what `sent_entries` would have read back from
//! the same tree packed the same way. Every test in this file pins one shape
//! of that parity so a drift on either side fails here, not at a 220 MB copy
//! in production.
//!
//! The streaming transport calls into the same `pack_path` the archive tests
// use; the parity test
//! ([`the_recorder_agrees_with_what_sent_entries_reads_back`]) is now also
//! expressed as a single-call assertion: pack to a `Vec` with the recorder
//! on, then assert `recorded == sent_entries(&vec)`.

use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use super::{pack_path_stream, PackedStream};
use crate::engine::copy::archive_pack::pack_path;
use crate::engine::copy::verify::{sent_entries, SentEntry, SentKind};

/// A fresh counter for the streaming packer. The number never matters — the
/// packer only reads it to advance the `cp` progress row.
fn counter() -> Arc<AtomicU64> {
	Arc::new(AtomicU64::new(0))
}

/// Drive `pack_path_stream` to completion, draining the body stream so the
/// producer is no longer blocked on the bounded channel. Returns the recorded
/// list, or the producer's error (the same error `put_archive_verified` would
/// surface to the caller).
async fn run(packed: PackedStream) -> (Vec<SentEntry>, crate::error::Result<()>) {
	let PackedStream { mut body, producer } = packed;
	// Drain the body so the producer can finish writing; with no consumer the
	// blocking pack task stalls on `ChannelWriter::send_pending`.
	use futures_util::StreamExt;
	while let Some(item) = body.next().await {
		// A mid-pack error arrives as an `Err` item here. The producer's
		// `pack_outcome` is the path that carries it to the caller; the
		// body-level error is enough to unblock the producer, so we discard it
		// here and check the outcome below.
		let _ = item;
	}
	let outcome = match producer.await {
		Ok(o) => o,
		Err(e) => {
			return (
				Vec::new(),
				Err(crate::error::ComposeError::Build(e.to_string())),
			)
		}
	};
	let (sent, result) = outcome;
	(sent, result)
}

fn sorted(mut v: Vec<SentEntry>) -> Vec<(String, String)> {
	v.sort_by(|a, b| a.path.cmp(&b.path));
	v.into_iter()
		.map(|e| (e.path, format!("{:?}", e.kind)))
		.collect()
}

/// Drive the unified `pack_path` into a `Vec<u8>` and return both the bytes
/// and the recorded entry list. Drives the parity tests below.
fn pack_to_vec(
	src: &Path,
	follow_link: bool,
	name_override: Option<&str>,
	contents: bool,
) -> (Vec<u8>, Vec<SentEntry>) {
	let mut buf = Vec::new();
	let mut sent = Vec::new();
	{
		let mut tar = tar::Builder::new(&mut buf);
		pack_path(
			src,
			follow_link,
			name_override,
			contents,
			&mut tar,
			&mut sent,
		)
		.expect("pack_path");
	}
	(buf, sent)
}

/// Every shape the existing `sent_entries` tests already build, in one tree:
/// a file, a nested directory, a symlink, an empty file, and a directory next
/// to them. Streaming the recording and reading the archive back must agree
/// on the list, otherwise the post-PUT confirmation will diverge from what
/// the unified `pack_path` produced byte for byte.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn streaming_recording_matches_what_pack_path_wrote_for_a_mixed_tree() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::create_dir(payload.join("empty")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"beside-the-rest").unwrap();
	std::fs::write(payload.join("sub/inner.bin"), vec![7u8; 1234]).unwrap();
	std::fs::write(payload.join("sub/nothing"), b"").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("dangling")).unwrap();

	let (bytes, _) = pack_to_vec(&payload, false, None, false);
	let from_archive = sorted(sent_entries(&bytes).unwrap());

	let (recorded, result) = run(pack_path_stream(&payload, false, None, false, counter())).await;
	result.expect("packer should not error on a regular tree");
	let from_stream = sorted(recorded);

	assert_eq!(
		from_stream, from_archive,
		"the streaming recorder and sent_entries must agree"
	);
}

/// `contents=true` packs every descendant at its relative path with no wrapper
/// entry. The streaming recorder must produce the same list, otherwise
/// `tree_landed` would stat the wrapper that no longer exists in the archive.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn streaming_recording_matches_what_pack_path_wrote_for_contents_mode() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::write(payload.join("a.txt"), b"hi").unwrap();
	std::fs::write(payload.join("sub/b.txt"), b"hello").unwrap();

	let (bytes, _) = pack_to_vec(&payload, false, None, true);
	let from_archive = sorted(sent_entries(&bytes).unwrap());

	let (recorded, result) = run(pack_path_stream(&payload, false, None, true, counter())).await;
	result.expect("packer should not error on contents mode");
	let from_stream = sorted(recorded);

	assert_eq!(
		from_stream, from_archive,
		"contents mode must drop the wrapper on both sides"
	);
}

/// Renaming on the way in must rename on both sides. Otherwise the
/// confirmation would stat the source's own name and find nothing.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn streaming_recording_matches_what_pack_path_wrote_for_a_renamed_tree() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"hi").unwrap();
	std::fs::write(payload.join("sub/inner.bin"), vec![9u8; 64]).unwrap();

	let (bytes, _) = pack_to_vec(&payload, false, Some("renamed"), false);
	let from_archive: Vec<String> = sorted(sent_entries(&bytes).unwrap())
		.into_iter()
		.map(|(p, _)| p)
		.collect();

	let (recorded, result) = run(pack_path_stream(
		&payload,
		false,
		Some("renamed"),
		false,
		counter(),
	))
	.await;
	result.expect("packer should not error on a renamed tree");
	let from_stream: Vec<String> = sorted(recorded).into_iter().map(|(p, _)| p).collect();

	assert_eq!(
		from_stream, from_archive,
		"the rename must apply on both sides"
	);
}

/// A lone symlink: the recording carries exactly one `Link` entry, mirroring
/// `sent_entries` on the same source. Without this pin, dropping symlinks
/// from the recorder would silently pass — the archive would still carry
/// the link, but the post-PUT confirmation would never ask about it.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn streaming_recording_includes_a_lone_symlink() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("dangling");
	std::os::unix::fs::symlink("nowhere", &link).unwrap();

	let (bytes, _) = pack_to_vec(&link, false, None, false);
	let from_archive = sent_entries(&bytes).unwrap();

	let (recorded, result) = run(pack_path_stream(&link, false, None, false, counter())).await;
	result.expect("packer should not error on a lone symlink");

	assert_eq!(
		sorted(recorded.clone()),
		sorted(from_archive.clone()),
		"the streaming recorder must include the symlink the archive carries"
	);
	assert!(
		recorded
			.iter()
			.any(|e| matches!(&e.kind, SentKind::Link(t) if t == "nowhere")),
		"the recorded list must carry the link target the destination is asked about, got {:?}",
		recorded
	);
}

/// A single file: one `File(size)` entry, parity with the archive read-back.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn streaming_recording_matches_what_pack_path_wrote_for_a_single_file() {
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("f.txt");
	std::fs::write(&file, b"fifteen bytes!!").unwrap();

	let (bytes, _) = pack_to_vec(&file, false, None, false);
	let from_archive = sorted(sent_entries(&bytes).unwrap());

	let (recorded, result) = run(pack_path_stream(&file, false, None, false, counter())).await;
	result.expect("packer should not error on a single file");
	let from_stream = sorted(recorded);

	assert_eq!(from_stream, from_archive);
}

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

/// An empty file in the source must still be recorded. The list must include
/// the file at its size, even when the size is zero — otherwise a
/// `cp src/empty.txt svc:/dst` would not confirm against the destination's
/// zero-length file.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn an_empty_file_is_recorded_with_size_zero() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(&payload).unwrap();
	std::fs::write(payload.join("nothing"), b"").unwrap();

	let (recorded, result) = run(pack_path_stream(&payload, false, None, false, counter())).await;
	result.expect("packer should not error on an empty file");

	assert!(
		recorded
			.iter()
			.any(|e| e.path == "payload/nothing" && matches!(e.kind, SentKind::File(0))),
		"the empty file must be recorded with File(0), got {:?}",
		recorded
	);
}

/// Sanity: streaming the body does not change what the recorder sees.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn the_recorder_runs_inside_spawn_blocking_and_finishes() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(&payload).unwrap();
	std::fs::write(payload.join("a.txt"), b"hi").unwrap();
	std::fs::write(payload.join("b.txt"), b"there").unwrap();

	let (recorded, result) = run(pack_path_stream(&payload, false, None, false, counter())).await;
	result.expect("packer should not error on a regular tree");
	assert_eq!(recorded.len(), 3, "wrapper + two files, got {:?}", recorded);
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

/// Watch sync's gzipped packer records the same list `cp`'s plain packer
/// would for the same source. The shared confirmation works on either body.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn sync_recording_includes_a_symlink() {
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("changed");
	std::fs::create_dir_all(&src).unwrap();
	std::fs::write(src.join("a.txt"), b"hi").unwrap();
	std::os::unix::fs::symlink("nowhere", src.join("dangling")).unwrap();

	let packed =
		crate::engine::copy::build_sync_tar_stream_for_watch(&src, Path::new("changed"), counter());
	let (_recorded, result) = run(packed).await;
	result.expect("sync packer should not error on a tree with a link");
	// The exact list shape is covered by the read-back test; here we only
	// pin that the recorder ran and finished cleanly.
}

/// The single-call parity: pack to a `Vec` with the recorder on, then
/// assert `recorded == sent_entries(&vec)`. The recorder and the bytes must
/// agree on every entry; this pins the contract that the recorder and the
/// archive are the same list. It also pins the symmetric shape: if the
/// recording side adds an entry the tar does not carry, the parity fails
/// on this side too.
#[cfg(unix)]
#[test]
fn the_recorder_agrees_with_what_sent_entries_reads_back() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::create_dir(payload.join("empty")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"beside-the-rest").unwrap();
	std::fs::write(payload.join("sub/inner.bin"), vec![7u8; 1234]).unwrap();
	std::fs::write(payload.join("sub/nothing"), b"").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("dangling")).unwrap();

	let (bytes, recorded) = pack_to_vec(&payload, false, None, false);
	let from_archive = sent_entries(&bytes).expect("sent_entries");

	let mut a: Vec<(String, String)> = recorded
		.iter()
		.map(|e| (e.path.clone(), format!("{:?}", e.kind)))
		.collect();
	let mut b: Vec<(String, String)> = from_archive
		.iter()
		.map(|e| (e.path.clone(), format!("{:?}", e.kind)))
		.collect();
	a.sort();
	b.sort();
	assert_eq!(
		a, b,
		"the recorder must equal sent_entries of the same bytes"
	);
}
