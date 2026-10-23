//! `cp` from a service container to the host.
//!
//! One direction, and the routing decision that makes it cheap: an existing
//! directory streams the archive body straight into the guarded extractor,
//! anything else buffers first. The classification of the destination and the
//! guard it consults are the same two questions [`destination`]
//! answers, so the caller and the extractor agree on what "a directory here"
//! means.

use std::path::Path;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::super::Engine;
use super::extract::archive::extract_archive;
use super::extract::destination;
use super::extract::stream;
use super::CpByteCounter;
use super::CpOptions;

/// Upper bound on a container→host `cp` archive buffered in memory. Without it a
/// hostile or huge container path would OOM the CLI. Generous (covers ordinary
/// file/dir copies); larger transfers should use `podman cp` directly.
const MAX_CP_ARCHIVE_BYTES: usize = 1024 * 1024 * 1024;

/// What `dst` looks like for the container→host cp routing decision.
///
/// Three cases drive [`Engine::cp_from_container`]:
/// - `Directory`: an existing real directory; the streaming extractor can
///   pipe the archive body straight into it without buffering.
/// - `Symlink`: a destination with a symlink at any component of its path,
///   the last one included, or with a component that could not be inspected
///   (refused the same way). `Path::is_dir` would follow the link and report
///   a directory, so the routing used to take the streaming branch and the
///   bytes landed in the link target rather than the named destination. The
///   same class of bug #1736 closed inside `extract_archive`; this is that
///   fix mirrored at the call site that picks between streaming and
///   buffering. #1736 looked at the last component only, and #1764 extended
///   it to the whole path.
/// - `NotADirectory`: a missing path (the buffered `extract_archive`
///   branch will create it) or an existing non-directory (the same
///   branch will land the single entry there).
pub(in crate::engine) enum CpDestinationKind {
	Directory,
	Symlink,
	NotADirectory,
}

/// Classify `dst` for the cp routing. `symlink_metadata` is used rather
/// than `is_dir`/`exists` so a destination that is a symlink is reported
/// as a symlink, not the directory it points at. Without this, the
/// streaming branch would extract into the link target rather than the
/// named destination (#1736 + the call-site follow-up).
///
/// `symlink_metadata` alone only answers for the last component, so the
/// whole path goes through
/// [`destination::destination_refusal`] first (#1764). That module
/// also records what the check does not close.
///
/// After the walk accepted the destination, the metadata is read through
/// [`destination::destination_metadata`] so a trusted root link whose
/// target IS the destination is followed (the walk would have let it
/// through); every other link still reads as a link.
pub(in crate::engine) fn cp_destination_kind(dst: &Path) -> CpDestinationKind {
	if destination::destination_refusal(dst).is_some() {
		return CpDestinationKind::Symlink;
	}
	match destination::destination_metadata(dst) {
		Ok(meta) if meta.file_type().is_symlink() => CpDestinationKind::Symlink,
		Ok(meta) if meta.is_dir() => CpDestinationKind::Directory,
		_ => CpDestinationKind::NotADirectory,
	}
}

impl Engine {
	pub(super) async fn cp_from_container(
		&self,
		file: &ComposeFile,
		service_name: &str,
		container_path: &str,
		dst: &Path,
		opts: &CpOptions,
		progress: CpByteCounter,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = self
			.live_replica_name_at(service_name, service, opts.index)
			.await?;

		let path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(container_path),
		);
		let resp = self
			.client
			.get_stream(&path)
			.await
			.map_err(ComposeError::Podman)?;
		let dst = dst.to_path_buf();

		// Two destination shapes, and only one of them can be streamed.
		//
		// An existing directory goes straight to `extract_tar_guarded`, which
		// walks the archive once, so the body can be piped into it and nothing
		// accumulates. That is also the shape that moves bulk data (`cp
		// svc:/var/lib/data ./backup/`), which is why it is the one worth
		// streaming.
		//
		// Any other destination goes through `extract_archive`, which reads the
		// archive twice: `archive_contains_dir` decides whether the destination
		// names a file or a directory to create, and only then does it extract.
		// A stream cannot be rewound, so that path still buffers. Making it
		// single-pass means changing what `cp` does with an ambiguous
		// destination, which is a behaviour decision rather than a memory one.
		//
		// The classification goes through `symlink_metadata` rather than
		// `is_dir` so a destination that is a symlink is reported as a symlink
		// rather than the directory it points at; the streaming branch would
		// otherwise extract into the link target. Same class of bug #1736
		// closed inside `extract_archive`, mirrored here at the routing site.
		match cp_destination_kind(&dst) {
			CpDestinationKind::Symlink => {
				return Err(destination::refusal_for(&dst));
			}
			CpDestinationKind::Directory => {
				return stream::extract_streamed(
					resp,
					dst,
					MAX_CP_ARCHIVE_BYTES as u64,
					Some(progress),
				)
				.await;
			}
			CpDestinationKind::NotADirectory => {
				// Fall through: `extract_archive` will either create a fresh
				// directory (directory source) or land the single entry at
				// exactly `dst` (single file source).
			}
		}

		// Cap the buffered archive so a huge/hostile container path cannot OOM
		// the CLI (the streaming `get_stream` path bypasses the client's own
		// cap). Kept as `Bytes` rather than `.to_vec()`: the extractor takes
		// `&[u8]`, which `Bytes` derefs to, and a `.to_vec()` here would hold a
		// second copy alive beside the first.
		let tar_bytes: Bytes = Limited::new(resp.into_body(), MAX_CP_ARCHIVE_BYTES)
			.collect()
			.await
			.map_err(|_| {
				ComposeError::Unsupported(format!(
					"cp: container archive exceeds {MAX_CP_ARCHIVE_BYTES} bytes; \
					 copy fewer files or use `podman cp` for very large transfers"
				))
			})?
			.to_bytes();
		progress.add(tar_bytes.len() as u64);

		tokio::task::spawn_blocking(move || extract_archive(&tar_bytes, &dst))
			.await
			.map_err(|e| ComposeError::Build(e.to_string()))??;

		Ok(())
	}
}
