//! Confirming a tree upload that the runtime never answered.
//!
//! A single file is confirmed by its size and its kind at the destination. A
//! directory has no size that says anything about its children, so #1777 had
//! every directory copy against Podman 6 reported as failed, landed or not.
//! The question asked here is the same one, put to each entry: is every
//! regular file of the archive that was sent at the destination with its
//! size, every directory there as a directory, and every symbolic link there
//! as a symlink pointing at the target the archive carried.
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
//! - Symbolic links (`SentKind::Link(target)`): must be a symlink at the
//!   destination AND its `linkTarget` must equal the target the archive
//!   carried. Confirming on the symlink bit alone lets any pre-existing link
//!   at the destination satisfy an upload whose target was something else;
//!   the target is read out of `X-Docker-Container-Path-Stat` (`PathStat`'s
//!   `link_target`, the libpod `linkTarget` field) and compared. If the stat
//!   carries no target the entry is unconfirmed, which fails closed rather
//!   than guessing. The stat for a dangling link on Podman 5.7.0 is carried
//!   on the 404 response in the stat header, which `head_path_stat` threw
//!   away; `head_path_stat_even_if_missing` reads it back so a cut stream
//!   cannot be confirmed against the link that was already there.
//! - Hard links, FIFOs, char/block devices and anything else: an entry of a
//!   kind that cannot be asked about through the archive stat makes the
//!   archive unverifiable. `sent_entries` returns an error naming the type,
//!   the same way a non-UTF-8 path already does, so the tree answer is "not
//!   landed" and the upload fails closed.
//!
//! ## What the file confirmation still cannot see
//!
//! libpod's stat carries name, size, mode, mtime and (for a link) the target;
//! it does not carry a checksum. A failed upload over regular files of the
//! same length is therefore still reported as landed: the size comparison is
//! the only evidence the stat endpoint returns, and a file the destination
//! already held at that size passes the check unchanged. The link path narrows
//! the false positive a step further (the destination link has to point at
//! the same target as the one sent), but a link whose target was already
//! there pointing the same way passes too. Documented; not closed.
//!
//! ## Paths
//!
//! Every path is taken from the tar as UTF-8. An entry whose path has a byte
//! that is not valid UTF-8 makes `sent_entries` return an error, which makes
//! `tree_landed` answer "not landed". A copy of such a tree against Podman 6
//! with a dropped response is then reported as failed rather than confirmed
//! against the lossy-rewritten name. No lossy conversion anywhere in this file.
//! The same applies to a symlink entry whose target is not valid UTF-8: the
//! archive would have to be rewritten to compare it, and a confirmation that
//! silently rewrote the target would be the same lossy answer on the link
//! half.

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SentKind {
	/// A regular file of this many bytes.
	File(u64),
	Dir,
	/// A symbolic link whose target is the string the archive carried.
	/// `entry_landed` matches it against the destination's `link_target`,
	/// which is libpod's `linkTarget` field, and refuses any stat that
	/// does not carry a target.
	Link(String),
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
/// `head_path_stat_even_if_missing` so the stat is read on the 404 too. The
/// target of every symlink is read out of the tar's linkname header and
/// carried in `SentKind::Link`, because that is the question the destination
/// is asked.
///
/// Hard links, FIFOs, devices and anything else the archive stat cannot be
/// asked about make the archive unverifiable: this function returns an error
/// naming the offending entry type, the same way a non-UTF-8 path already
/// does, so `tree_landed` answers "not landed" and the copy fails closed
/// rather than confirming on an entry the destination was never asked about.
///
/// An entry whose path is not valid UTF-8 makes the function return an error.
/// `tree_landed` reads that as "not landed", so a copy of such a tree against
/// Podman 6 with a dropped response is reported as failed rather than
/// confirmed against a lossy-rewritten name. So is an entry whose path is the
/// extraction directory itself (`.`), which the caller confirmed before
/// uploading. A symlink entry whose target is not valid UTF-8 is treated the
/// same way: silently rewriting the target would compare against a lossy
/// string, so the tree answer is "not landed" instead.
pub(super) fn sent_entries(gz_tar: &[u8]) -> Result<Vec<SentEntry>> {
	let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(gz_tar));
	let mut sent = Vec::new();
	for entry in archive.entries().map_err(ComposeError::Io)? {
		let entry = entry.map_err(ComposeError::Io)?;
		let entry_type = entry.header().entry_type();
		let kind = match entry_type {
			tar::EntryType::Regular | tar::EntryType::Continuous => SentKind::File(entry.size()),
			tar::EntryType::Directory => SentKind::Dir,
			tar::EntryType::Symlink => {
				let target = entry
					.link_name()
					.map_err(ComposeError::Io)?
					.ok_or_else(|| {
						ComposeError::Build("cp: archive link entry has no target".into())
					})?;
				let target = target.to_str().ok_or_else(|| {
					ComposeError::Build("cp: archive link target is not UTF-8".into())
				})?;
				SentKind::Link(target.to_string())
			}
			_ => {
				return Err(ComposeError::Build(format!(
					"cp: archive entry of unverified type {entry_type:?} cannot be confirmed"
				)));
			}
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
/// there, so the size comparison on its own is not enough. A symbolic link
/// must be a symlink at the destination AND its `link_target` must equal the
/// target the archive carried; a stat that carries no target cannot answer
/// the question and the entry is unconfirmed (the caller treats it as
/// failure).
pub(super) fn entry_landed(sent: &SentKind, post: Option<&PathStat>) -> bool {
	let is_dir = |stat: &PathStat| stat.mode & (1 << 31) != 0;
	let is_regular = |stat: &PathStat| stat.mode & MODE_TYPE == 0;
	let is_symlink = |stat: &PathStat| stat.mode & (1 << 27) != 0;
	match sent {
		SentKind::File(size) => post.is_some_and(|stat| is_regular(stat) && stat.size == *size),
		SentKind::Dir => post.is_some_and(is_dir),
		SentKind::Link(target) => post.is_some_and(|stat| {
			is_symlink(stat) && stat.link_target.as_deref() == Some(target.as_str())
		}),
	}
}

impl Engine {
	/// Whether every file, directory and symbolic link of `gz_tar` is at `dir`
	/// in `container`.
	///
	/// An archive that holds a hard link, a FIFO, a device, a block/char
	/// device or anything else that cannot be asked about through the
	/// archive stat is unverifiable: `sent_entries` errors on the offending
	/// entry type and this function answers "not landed", so the upload
	/// fails closed rather than confirming against an entry the destination
	/// was never asked about.
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
	/// The residual false positive is the size-only one: libpod's stat has no
	/// checksum, so a failed upload over files of the same length is still
	/// reported as landed, and a pre-existing link pointing at the same target
	/// is reported as landed too. Documented in the module doc.
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
				SentKind::Link(_) => self.client.head_path_stat_even_if_missing(&stat_path).await,
				_ => self.client.head_path_stat(&stat_path).await,
			};
			match stat {
				Ok(post) if entry_landed(&entry.kind, post.as_ref()) => {}
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
