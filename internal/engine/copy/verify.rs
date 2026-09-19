//! Confirming a tree upload that the runtime never answered.
//!
//! A single file is confirmed by its size at the destination. A directory has
//! no size that says anything about its children, so #1777 had every directory
//! copy against Podman 6 reported as failed, landed or not. The question asked
//! here is the same one, put to each entry: is every regular file of the
//! archive that was sent at the destination with its size, every directory
//! there as a directory, and every symbolic link there as a symlink.
//!
//! The expectation is read back out of the archive that went over the wire,
//! not from a second walk of the source, so it cannot describe a tree other
//! than the one uploaded.
//!
//! One stat per entry, about 10 ms each against Podman 5.7.0 on 2026-09-18, and
//! only after a dropped response. The one request that lists a destination
//! recursively is the archive GET, which returns the contents too and so costs
//! what was already there instead of what was sent; that is why it is not used.
//!
//! ## What is and is not checked
//!
//! - Regular files (`SentKind::File(size)`): must be a regular file at the
//!   destination with the same size. A directory stats at 4096 bytes on most
//!   filesystems, so the regular-file check rejects anything that has a Go
//!   `os.ModeType` bit set, not just `ModeDir`; an empty file uploaded over an
//!   existing named pipe would otherwise be confirmed by the unchanged pipe.
//! - Directories (`SentKind::Dir`): must be a directory at the destination.
//! - Symbolic links (`SentKind::Link`): must be a symlink at the destination,
//!   whatever the target. The stat for a dangling link on Podman 5.7.0 was
//!   carried on the 404 response in the `X-Docker-Container-Path-Stat` header,
//!   which `head_path_stat` threw away; `head_path_stat_even_if_missing` reads
//!   it back so a cut stream cannot be confirmed against the link that was
//!   already there.
//! - Hard links, FIFOs, devices, block/char devices and everything else:
//!   left out of the expectation. `sent_entries` filters them out, and
//!   `tree_landed` is told nothing about them.
//!
//! ## Paths
//!
//! Every path is taken from the tar as UTF-8. An entry whose path has a byte
//! that is not valid UTF-8 makes `sent_entries` return an error, which makes
//! `tree_landed` answer "not landed". A copy of such a tree against Podman 6
//! with a dropped response is then reported as failed rather than confirmed
//! against the lossy-rewritten name. No lossy conversion anywhere in this file.

use bytes::Bytes;

use crate::error::{ComposeError, Result};
use crate::libpod::client::PathStat;
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;

use super::super::Engine;
use super::join_archive_path;

/// Every `os.ModeType` bit Go reports on the stat endpoint.
///
/// `ModeDir` (1<<31), `ModeSymlink` (1<<27), `ModeDevice` (1<<26), `ModeNamedPipe`
/// (1<<25), `ModeSocket` (1<<24), `ModeCharDevice` (1<<21) and `ModeIrregular`
/// (1<<19) per Go's `io/fs`. A regular file has none of them set, which is the
/// distinction the regular-file confirmation needs.
const MODE_TYPE: u64 =
	(1 << 31) | (1 << 27) | (1 << 26) | (1 << 25) | (1 << 24) | (1 << 21) | (1 << 19);

/// What an uploaded entry must be at the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SentKind {
	/// A regular file of this many bytes.
	File(u64),
	Dir,
	/// A symbolic link, the destination need only be a symlink.
	Link,
}

/// One entry of the uploaded archive that the destination can be asked about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SentEntry {
	/// Relative to the directory the archive was extracted at, `/`-separated,
	/// without a leading `./` or a trailing `/`.
	pub(super) path: String,
	pub(super) kind: SentKind,
}

/// The regular files, directories and symbolic links in a gzipped tar, in
/// archive order.
///
/// Symbolic links are kept here even though `head_path_stat` cannot ask about
/// them: on Podman 5.7.0 the 404 for a dangling link still carried the
/// `X-Docker-Container-Path-Stat` header, and the runtime is asked through
/// `head_path_stat_even_if_missing` so the stat is read on the 404 too.
///
/// Hard links, FIFOs, devices and anything else are left out, deliberately;
/// they are not representable here, and an archive that ends up holding only
/// such entries yields an empty expectation, which fails closed.
///
/// An entry whose path is not valid UTF-8 makes the function return an error.
/// `tree_landed` reads that as "not landed", so a copy of such a tree against
/// Podman 6 with a dropped response is reported as failed rather than
/// confirmed against a lossy-rewritten name. So is an entry whose path is the
/// extraction directory itself (`.`), which the caller confirmed before
/// uploading.
pub(super) fn sent_entries(gz_tar: &[u8]) -> Result<Vec<SentEntry>> {
	let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(gz_tar));
	let mut sent = Vec::new();
	for entry in archive.entries().map_err(ComposeError::Io)? {
		let entry = entry.map_err(ComposeError::Io)?;
		let kind = match entry.header().entry_type() {
			tar::EntryType::Regular | tar::EntryType::Continuous => SentKind::File(entry.size()),
			tar::EntryType::Directory => SentKind::Dir,
			tar::EntryType::Symlink => SentKind::Link,
			_ => continue,
		};
		let path = entry.path().map_err(ComposeError::Io)?;
		let mut names = Vec::new();
		for component in path.components() {
			match component {
				std::path::Component::Normal(name) => {
					let name = name.to_str().ok_or_else(|| {
						ComposeError::Build("cp: archive entry path is not UTF-8".into())
					})?;
					names.push(name.to_string());
				}
				_ => continue,
			}
		}
		if names.is_empty() {
			continue;
		}
		sent.push(SentEntry {
			path: names.join("/"),
			kind,
		});
	}
	Ok(sent)
}

/// Whether the destination's stat is the entry that was sent.
///
/// A regular file must match in size and have no Go `os.ModeType` bit set: a
/// directory, a symlink, a named pipe, a socket, a device or any other
/// non-regular kind reports a size that says nothing about what the upload put
/// there, so the size comparison on its own is not enough.
pub(super) fn entry_landed(sent: SentKind, post: Option<&PathStat>) -> bool {
	let is_dir = |stat: &PathStat| stat.mode & (1 << 31) != 0;
	let is_regular = |stat: &PathStat| stat.mode & MODE_TYPE == 0;
	let is_symlink = |stat: &PathStat| stat.mode & (1 << 27) != 0;
	match sent {
		SentKind::File(size) => post.is_some_and(|stat| is_regular(stat) && stat.size == size),
		SentKind::Dir => post.is_some_and(is_dir),
		SentKind::Link => post.is_some_and(is_symlink),
	}
}

impl Engine {
	/// Whether every file, directory and symbolic link of `gz_tar` is at `dir`
	/// in `container`.
	///
	/// Hard links, FIFOs, devices and anything else are not asked about (see
	/// the module doc), and an archive that ends up holding only such entries
	/// yields an empty expectation, which fails closed.
	///
	/// Entries are asked about last to first. Extraction is sequential, so a
	/// stream that was cut loses its tail, and this way round a truncated
	/// upload is refused on the first stat instead of the last.
	///
	/// Symbolic links go through `head_path_stat_even_if_missing` rather than
	/// `head_path_stat`; a dangling link answers 404 with the stat header on
	/// Podman 5.7.0, and `head_path_stat` would throw that stat away. A regular
	/// file or directory that answers 404 (the link was cut and the upload
	/// failed) returns `None` either way, so the dispatch does not matter for
	/// them; links are the only kind that benefit.
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
			let stat = match entry.kind {
				SentKind::Link => self.client.head_path_stat_even_if_missing(&stat_path).await,
				_ => self.client.head_path_stat(&stat_path).await,
			};
			match stat {
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
