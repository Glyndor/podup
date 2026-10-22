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

use super::uploaded_entry_kind;
use crate::engine::fake_podman::{self, FakePodman, FakeReply};
use crate::engine::Engine;
use crate::error::ComposeError;
use crate::libpod::urlencoded;

const CONTAINER: &str = "proj-web-1";

/// One path in the fake container.
#[derive(Clone)]
pub(super) enum OnDisk {
	File(u64),
	Dir,
	/// A symbolic link with its target as written in the tar; the stat
	/// header's `linkTarget` is the target normalised against the
	/// directory the link lives in (the way Podman 5.7.0 reports it).
	Link {
		size: u64,
		target: String,
	},
	/// A named pipe; the stat header carries size 0 and the
	/// `os.ModeNamedPipe` mode bit (1<<25 | 0o644).
	Fifo,
	/// There, but the runtime fails the stat with a 500.
	Unreadable,
}

/// How the fake answers the archive PUT.
#[derive(Clone, Copy)]
pub(super) enum Put {
	/// Accept the body and close without a response: Podman 6.
	HangsUp,
	/// A normal response with this status: Podman 5.
	Answers(u16),
}

/// Podman 5.7.0 normalises `linkTarget` lexically against the directory
/// the link lives in: a relative target is joined to the parent of the
/// link's path and `.` / `..` are collapsed; an absolute target is left
/// alone. The fake mirrors that so the link confirmation can compare the
/// sent target against the value the runtime would actually return.
pub(super) fn normalize_link_target_for_fake(target: &str, link_path: &str) -> String {
	if target.starts_with('/') {
		return target.to_string();
	}
	let parent = link_path
		.rfind('/')
		.map(|idx| if idx == 0 { "/" } else { &link_path[..idx] })
		.unwrap_or("/");
	let joined = if parent == "/" {
		format!("/{}", target)
	} else {
		format!("{}/{}", parent, target)
	};
	crate::engine::copy::verify::normalize_for_test(&joined)
}

pub(super) fn stat_header(path: &str, entry: &OnDisk) -> String {
	let name = path.rsplit('/').next().unwrap_or_default();
	let (size, mode, is_dir, link_target) = match entry {
		OnDisk::File(size) => (*size, 420u64, false, String::new()),
		OnDisk::Dir => (4096, 2_147_484_141, true, String::new()),
		OnDisk::Link { size, target } => (
			*size,
			(1u64 << 27) | 0o777,
			false,
			normalize_link_target_for_fake(target, path),
		),
		OnDisk::Fifo => (0, (1u64 << 25) | 0o644, false, String::new()),
		OnDisk::Unreadable => unreachable!("answered with a 500, not a stat"),
	};
	let json = format!(
		r#"{{"name":"{name}","size":{size},"mode":{mode},"mtime":"2026-09-18T19:50:59.194580835-05:00","isDir":{is_dir},"linkTarget":"{link_target}"}}"#
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
pub(super) fn runtime_full(
	put: Put,
	disk: &[(&str, OnDisk)],
	link_stat_on_404: bool,
) -> FakePodman {
	let disk: Vec<(String, OnDisk)> = disk
		.iter()
		.map(|(p, e)| ((*p).to_string(), e.clone()))
		.collect();
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
				OnDisk::Link { .. } if link_stat_on_404 => FakeReply::Headers(
					404,
					vec![("X-Docker-Container-Path-Stat", stat_header(path, entry))],
				),
				_ => FakeReply::Headers(
					200,
					vec![("X-Docker-Container-Path-Stat", stat_header(path, entry))],
				),
			})
			.unwrap_or(FakeReply::Headers(404, Vec::new())),
		_ => FakeReply::Body(404, r#"{"message":"not found"}"#.into()),
	})
}

/// As [`runtime_full`], with the Podman 5.7.0 link-stat-on-404 behaviour
/// (the default for every pre-existing test, which carries no links).
pub(super) fn runtime(put: Put, disk: &[(&str, OnDisk)]) -> FakePodman {
	runtime_full(put, disk, true)
}

pub(super) fn engine_for(fake: &FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

/// `payload/` with a file at the top, a file one level down and an empty
/// directory, which between them are every kind of entry a tree upload has to
/// account for.
pub(super) fn payload_tree(root: &Path) -> std::path::PathBuf {
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

/// Upload `src` to `/tmp` under `entry`, exactly as `cp` does it. Drives the
/// streaming packer the production path uses (#1844), so the fake socket sees
/// the same body shape hyper would see against Podman.
pub(super) async fn upload(
	fake: &FakePodman,
	src: &Path,
	entry: &str,
	rename: Option<&str>,
) -> crate::error::Result<()> {
	let packed = super::pack::pack_path_stream(
		src,
		false,
		rename,
		false,
		super::progress::ByteCounter::new().inner().clone(),
	);
	engine_for(fake)
		.put_archive_verified(
			CONTAINER,
			"/tmp",
			entry,
			packed,
			uploaded_entry_kind(src, false),
		)
		.await
}

pub(super) fn assert_unconfirmed(result: crate::error::Result<()>, case: &str) {
	match result {
		Err(ComposeError::Copy(msg)) => assert!(
			msg.contains("could not be confirmed"),
			"{case}: refused, but not as an unconfirmed upload: {msg}"
		),
		Err(ComposeError::Build(msg)) => assert!(
			msg.contains("cp:")
				|| msg.contains("unverified")
				|| msg.contains("Fifo")
				|| msg.contains("permission"),
			"{case}: the streaming packer refused at pack time, but the message does not \
			 carry the original cause: {msg}"
		),
		other => panic!("{case}: must be reported as an unconfirmed upload, got {other:?}"),
	}
}

/// Extract the inner detail the caller wraps, for the assertion that the
/// user-facing error names the entry and the stat. Today the message reaches
/// the user via `ComposeError::Copy(_)`; this is the seam the tests read.
pub(super) fn copy_detail(result: crate::error::Result<()>, case: &str) -> String {
	match result {
		Err(ComposeError::Copy(msg)) => msg,
		other => panic!("{case}: must be reported as an unconfirmed copy, got {other:?}"),
	}
}

// Links and the non-regular kinds (FIFOs) live in their own file so this one
// stays under the line limit. A child module, so it reaches the fixtures above
// through `super::` without widening their visibility.
mod kinds;

mod entry_confirmation;
mod packing_errors;
mod put_responses;
mod tree_confirmation;
