//! `cp` from the host into a service container.
//!
//! One direction, and the two rules that make it match `docker cp`: what the
//! archive is named, and where it is PUT. Both are settled before a byte is
//! packed, because the entry name is what the post-PUT confirmation asks the
//! destination about.

use std::path::Path;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::super::Engine;
use super::pack::transport;
use super::upload::verify::SentKind;
use super::CpByteCounter;
use super::CpOptions;

/// What the destination entry must look like for a single-entry upload to have
/// landed, or `None` when there is nothing comparable.
///
/// A regular file source expects `SentKind::File(size)`, where the size is the
/// file's actual length on disk. The kind is part of the comparison (an empty
/// file over an unchanged zero-length FIFO would otherwise pass on size
/// alone). A symlink at the source without `-L/--follow-link` expects
/// `SentKind::Link(target)`, where `target` is what the tar will carry as
/// the link name (read out of the host symlink with `fs::read_link`). The
/// destination is asked about that target after Podman normalises it the same
/// way it normalises `linkTarget`, and the byte length of the target is
/// checked against `stat.size` independently. A directory source returns `None`
/// because its own size says nothing about its children; that case is
/// confirmed entry by entry (`upload::verify::tree_landed`), never on this.
/// Anything else stays unverifiable and fails closed rather than confirming
/// on the wrong kind.
///
/// Extracted so it is reachable from a test. Inside the async upload it was
/// covered only by running against a real container, and a mutation
/// replacing the regular-file length with a constant survived the whole
/// unit suite.
pub(in crate::engine) fn uploaded_entry_kind(src: &Path, follow_link: bool) -> Option<SentKind> {
	let meta = if follow_link {
		std::fs::metadata(src).ok()?
	} else {
		std::fs::symlink_metadata(src).ok()?
	};
	let kind = meta.file_type();
	if kind.is_symlink() && !follow_link {
		// `read_link` returns the link target as the host filesystem stores
		// it; that is the same bytes `pack_path` will put in the tar's
		// linkname field, so it is the value the destination's stat will
		// be asked to match. A path that is not valid UTF-8 makes the
		// entry unverifiable on the host side (we cannot normalise what
		// we cannot name), and the archive PUT would refuse it anyway.
		let target = std::fs::read_link(src).ok()?;
		let target = target.to_str().map(str::to_string)?;
		Some(SentKind::Link(target))
	} else if kind.is_file() {
		Some(SentKind::File(meta.len()))
	} else {
		None
	}
}

/// Join a container directory and an entry name into one path, without doubling
/// the separator when the directory already ends in `/` (so root `/` yields
/// `/name`, not `//name`). Pure so the join is unit-tested without a container.
pub(super) fn join_archive_path(dir: &str, entry: &str) -> String {
	if dir.ends_with('/') {
		format!("{dir}{entry}")
	} else {
		format!("{dir}/{entry}")
	}
}

impl Engine {
	/// Push a host file or directory into a service container.
	///
	/// # Concurrency contract: read before touching the two HEAD + PUT sequence
	///
	/// `cp_to_container` issues two `HEAD /archive` requests with the PUT
	/// between them, so a concurrent mutation in the window could land a
	/// successful-but-wrong-state PUT. The two callers in the codebase are
	/// the CLI `cp` subcommand and the `watch` sync path, both of which are
	/// called only while the per-project lock ([`crate::engine::project::lock`]) is
	/// held by the mutating stage: `lock_project` serialises a single
	/// `podup` process against any other `podup` process working on the same
	/// project, closing the **cross-invocation** case.
	///
	/// The **within-invocation** case, a foreign actor (a manual
	/// `podman exec`, another compose stack on the same machine, the user
	/// running `podman cp` in another shell) mutating the destination
	/// between the two HEADs, is closed by libpod itself: the archive PUT
	/// extracts into a directory that we have just confirmed exists and is
	/// a directory, so a foreign `rm -rf` racing in is rejected by the
	/// second PUT, not silently succeeded. The `extract_stat_path` HEAD
	/// below is what makes that property hold; do not skip it.
	#[allow(clippy::too_many_arguments)]
	pub(super) async fn cp_to_container(
		&self,
		file: &ComposeFile,
		service_name: &str,
		src: &Path,
		container_path: &str,
		opts: &CpOptions,
		contents: bool,
		progress: CpByteCounter,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = self
			.live_replica_name_at(service_name, service, opts.index)
			.await?;

		// Match `docker cp` destination semantics. The libpod archive PUT extracts
		// the tar *at* a directory, so:
		//  - dest is an existing directory (or ends in `/`)  → copy the source in
		//    under its own name (PUT to the dest dir);
		//  - dest is anything else (a new name, or a file)   → rename the source to
		//    the dest's basename and PUT to the dest's parent.
		// Without this, `cp file svc:/path/newname` created `newname/` as a
		// directory holding the source instead of a file named `newname`.
		//
		// The `contents` branch (trailing `/.` on the host source) short-
		// circuits the rename: the archive holds no wrapper, and the PUT
		// destination IS the final location (whether it already exists or not).
		// That matches `podman cp host/. svc:/path/newname`, which lands the
		// contents directly at `/path/newname/`.
		let stat_path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(container_path),
		);
		let dest_is_dir = self.client.head_path_is_dir(&stat_path).await? == Some(true);

		let (extract_dir, rename) = if contents {
			// Contents land at the destination itself; libpod creates it if
			// it does not exist. No rename, no wrapper.
			(container_path.trim_end_matches('/').to_string(), None)
		} else if dest_is_dir || container_path.ends_with('/') {
			(container_path.trim_end_matches('/').to_string(), None)
		} else {
			let trimmed = container_path.trim_end_matches('/');
			let (parent, name) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
			let parent = if parent.is_empty() { "/" } else { parent };
			(parent.to_string(), Some(name.to_string()))
		};

		// Validate the extraction directory exists and is itself a directory before
		// PUTting the archive. Without this, libpod silently auto-creates a missing
		// parent chain (diverging from docker/podman `cp`, which error with "no such
		// directory"); and when a path component is a regular file the archive PUT
		// never gets a response, blocking the full READ_TIMEOUT window instead of
		// failing fast.
		let extract_stat_path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(&extract_dir),
		);
		match self.client.head_path_is_dir(&extract_stat_path).await? {
			Some(true) => {}
			Some(false) => {
				return Err(ComposeError::Copy(format!(
					"cp: not a directory: {extract_dir}"
				)));
			}
			None => {
				return Err(ComposeError::Copy(format!(
					"cp: no such directory: {extract_dir}"
				)));
			}
		}

		let src_buf = src.to_path_buf();
		let follow = opts.follow_link;
		let rename_for_pack = rename.clone();
		// Stream the tar bytes into a bounded channel rather than building
		// the whole archive into a `Vec<u8>` first (#1844). The recorded
		// entry list is what the post-PUT confirmation compares against
		// once the body bytes are gone.
		let packed = transport::pack_path_stream(
			&src_buf,
			follow,
			rename_for_pack.as_deref(),
			contents,
			progress.inner().clone(),
		);

		// Contents-packed archives have no wrapper entry: `tree_landed` walks
		// the recorded list and asks about each entry against the destination.
		// The existing `entry`-based confirmation is only meaningful when the
		// archive is wrapped under a single name, which `contents=true`
		// removes.
		let entry = if contents {
			String::new()
		} else {
			rename.clone().unwrap_or_else(|| {
				src.file_name()
					.map(|n| n.to_string_lossy().into_owned())
					.unwrap_or_default()
			})
		};
		let uploaded_kind = uploaded_entry_kind(src, follow);
		self.put_archive_verified(&container_name, &extract_dir, &entry, packed, uploaded_kind)
			.await
	}
}
