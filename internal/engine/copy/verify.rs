//! Confirming a tree upload that the runtime never answered.
//!
//! A single file is confirmed by its size at the destination. A directory has
//! no size that says anything about its children, so #1777 had every directory
//! copy against Podman 6 reported as failed, landed or not. The question asked
//! here is the same one, put to each entry: is every regular file of the
//! archive that was sent at the destination with its size, and every directory
//! there as a directory.
//!
//! The expectation is read back out of the archive that went over the wire,
//! not from a second walk of the source, so it cannot describe a tree other
//! than the one uploaded.
//!
//! One stat per entry, about 10 ms each against Podman 5.7.0 on 2026-09-18, and
//! only after a dropped response. The one request that lists a destination
//! recursively is the archive GET, which returns the contents too and so costs
//! what was already there instead of what was sent; that is why it is not used.

use bytes::Bytes;

use crate::error::{ComposeError, Result};
use crate::libpod::client::PathStat;
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;

use super::super::Engine;
use super::join_archive_path;

/// What an uploaded entry must be at the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SentKind {
	/// A regular file of this many bytes.
	File(u64),
	Dir,
}

/// One entry of the uploaded archive that the destination can be asked about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SentEntry {
	/// Relative to the directory the archive was extracted at, `/`-separated,
	/// without a leading `./` or a trailing `/`.
	pub(super) path: String,
	pub(super) kind: SentKind,
}

/// The regular files and directories in a gzipped tar, in archive order.
///
/// Links and special files are left out, deliberately: a dangling symlink
/// answered the stat with a 404 on Podman 5.7.0 (2026-09-18) although it was
/// there, so asking about a link would turn a copy that landed into a failure.
/// So is an entry whose path is the extraction directory itself (`.`), which
/// the caller confirmed before uploading.
pub(super) fn sent_entries(gz_tar: &[u8]) -> Result<Vec<SentEntry>> {
	let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(gz_tar));
	let mut sent = Vec::new();
	for entry in archive.entries().map_err(ComposeError::Io)? {
		let entry = entry.map_err(ComposeError::Io)?;
		let kind = match entry.header().entry_type() {
			tar::EntryType::Regular | tar::EntryType::Continuous => SentKind::File(entry.size()),
			tar::EntryType::Directory => SentKind::Dir,
			_ => continue,
		};
		let path = entry.path().map_err(ComposeError::Io)?;
		let path = path
			.components()
			.filter_map(|c| match c {
				std::path::Component::Normal(name) => Some(name.to_string_lossy()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join("/");
		if !path.is_empty() {
			sent.push(SentEntry { path, kind });
		}
	}
	Ok(sent)
}

/// Whether the destination's stat is the entry that was sent.
///
/// A file must not be a directory as well as match in size: a directory stats
/// at 4096 bytes on most filesystems, which is also a plausible file length.
pub(super) fn entry_landed(sent: SentKind, post: Option<&PathStat>) -> bool {
	// Go's `os.ModeDir`, the top bit of the mode libpod reports.
	let is_dir = |stat: &PathStat| stat.mode & (1 << 31) != 0;
	match sent {
		SentKind::File(size) => post.is_some_and(|stat| !is_dir(stat) && stat.size == size),
		SentKind::Dir => post.is_some_and(is_dir),
	}
}

impl Engine {
	/// Whether every file and directory of `gz_tar` is at `dir` in `container`.
	///
	/// Anything short of a full match is `false`: an archive that cannot be
	/// read back, one with nothing in it to ask about, a stat that fails, an
	/// entry that is absent or is not what was sent. Unknown must not become a
	/// guess, exactly as for a single file.
	///
	/// Entries are asked about last to first. Extraction is sequential, so a
	/// stream that was cut loses its tail, and this way round a truncated
	/// upload is refused on the first stat instead of the last.
	///
	/// Sequential on purpose. Podman 6 dropped responses under concurrency
	/// (#1339), and a dropped stat here is a landed copy reported as failed.
	///
	/// The residual false positive is the single-file one at tree size: an
	/// upload that failed over a tree whose entries already had these sizes.
	pub(super) async fn tree_landed(&self, container: &str, dir: &str, gz_tar: Bytes) -> bool {
		let read_back = tokio::task::spawn_blocking(move || sent_entries(&gz_tar))
			.await
			.map_err(|e| ComposeError::Build(e.to_string()))
			.and_then(|sent| sent);
		let sent = match read_back {
			Ok(sent) => sent,
			Err(e) => {
				tracing::debug!("cp: could not read back the uploaded archive: {e}");
				return false;
			}
		};
		if sent.is_empty() {
			return false;
		}
		for entry in sent.iter().rev() {
			let stat_path = format!(
				"{API_PREFIX}/containers/{}/archive?path={}",
				urlencoded(container),
				urlencoded(&join_archive_path(dir, &entry.path)),
			);
			match self.client.head_path_stat(&stat_path).await {
				Ok(post) if entry_landed(entry.kind, post.as_ref()) => {}
				Ok(post) => {
					tracing::debug!(
						"cp: {} in {dir} is not what was uploaded ({:?}): {post:?}",
						entry.path,
						entry.kind
					);
					return false;
				}
				Err(stat_err) => {
					tracing::debug!(
						"cp: could not re-verify {} in {dir} after an incomplete PUT: {stat_err}",
						entry.path
					);
					return false;
				}
			}
		}
		true
	}
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
