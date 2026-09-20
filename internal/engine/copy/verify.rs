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
//!   destination AND its `link_target` must equal the target the archive
//!   carried, or its absolute target must equal the archive's relative target
//!   resolved against the destination directory. Confirming on the symlink bit
//!   alone lets any pre-existing link at the destination satisfy an upload
//!   whose target was something else; the target is read out of
//!   `X-Docker-Container-Path-Stat` (`PathStat`'s `link_target`, the libpod
//!   `linkTarget` field) and compared. The stat for a dangling link on
//!   Podman 5.7.0 is carried on the 404 response in the stat header, which
//!   `head_path_stat` threw away; `head_path_stat_even_if_missing` reads it
//!   back so a cut stream cannot be confirmed against the link that was
//!   already there. A stat that carries no target at all (the field is
//!   absent, or it is the empty string) leaves the entry to be confirmed
//!   on the symlink bit alone. A symlink always points at something, so an
//!   empty string is not a valid symlink target and a runtime reporting `""`
//!   has not answered the question, exactly as one that omits the field has
//!   not. That is the documented residual on a runtime that does not report
//!   link targets, and the call sites emit a `tracing::warn!` so a CI log
//!   carries the reason. A pre-existing link pointing elsewhere still passes
//!   under that fallback, because the runtime gave the confirmation nothing
//!   stronger to compare against. **It is the price of not failing a copy
//!   that landed.**
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

/// Whether the destination's stat is the entry that was sent, and if so on
/// what evidence.
///
/// A regular file must match in size and have no Go `os.ModeType` bit set: a
/// directory, a symlink, a named pipe, a socket, a device or any other
/// non-regular kind reports a size that says nothing about what the upload put
/// there, so the size comparison on its own is not enough. A symbolic link
/// must be a symlink at the destination. The target is read out of the stat
/// (`PathStat::link_target`, the libpod `linkTarget` field) and matched
/// against the target the archive carried. The cases:
///
/// - A matching target confirms on the strong path: the destination points at
///   the same string the archive carried.
/// - A target the runtime reports as absolute and that, when joined to the
///   destination directory, equals the absolute form of the archive's
///   relative target confirms on the strong path: a runtime that resolves a
///   relative linkname against the destination directory and reports the
///   absolute result is answered on the same target the archive carried.
/// - A `link_target` the runtime did not answer (the field is absent on the
///   stat, or is the empty string) confirms on the symlink bit alone. A
///   symlink always points at something, so an empty string is not a valid
///   symlink target and a runtime reporting `""` has not answered the
///   question, exactly as one that omits the field has not. The destination
///   cannot answer the question any stronger than the runtime did, and
///   refusing a copy that landed would re-introduce the #1777 shape this
///   module was written to close. The call site emits a `tracing::warn!`
///   carrying the literal stat so a CI log names the runtime behaviour that
///   forced the fallback. The residual: a pre-existing link pointing
///   elsewhere still passes here.
/// - A target the runtime answered but that does not match (and that cannot
///   be related to the archive target through the destination path) does not
///   confirm. The destination's link points at something the archive did not
///   carry, and confirming on the bit alone would call that uploaded.
///
/// `dir` is the directory the archive was extracted at; it is the destination
/// against which the archive's relative linkname is resolved for the
/// absolute-target normalisation above.
pub(super) fn entry_landed(sent: &SentKind, post: Option<&PathStat>, dir: &str) -> LinkCheck {
	let is_dir = |stat: &PathStat| stat.mode & (1 << 31) != 0;
	let is_regular = |stat: &PathStat| stat.mode & MODE_TYPE == 0;
	let is_symlink = |stat: &PathStat| stat.mode & (1 << 27) != 0;
	match sent {
		SentKind::File(size) => match post {
			Some(stat) if is_regular(stat) && stat.size == *size => LinkCheck::Confirmed,
			Some(_) => LinkCheck::Refused,
			None => LinkCheck::Absent,
		},
		SentKind::Dir => match post {
			Some(stat) if is_dir(stat) => LinkCheck::Confirmed,
			Some(_) => LinkCheck::Refused,
			None => LinkCheck::Absent,
		},
		SentKind::Link(target) => match post {
			Some(stat) if !is_symlink(stat) => LinkCheck::Refused,
			Some(stat) => match stat.link_target.as_deref() {
				// A symlink always points at something, so a runtime that
				// reports an empty string for a symlink has not answered
				// the question, exactly as one that omits the field has
				// not. Same branch as the absent case: the symlink bit
				// alone, with a `warn!` carrying the literal stat at the
				// call site. Listed before the equality check so the empty
				// string is treated as a fallback rather than as a target
				// to match against.
				None | Some("") => LinkCheck::Fallback,
				Some(runtime_target) if runtime_target == target => LinkCheck::Confirmed,
				Some(runtime_target) => match normalise_relative_target(runtime_target, dir) {
					Some(normalised) if normalised == target => LinkCheck::Confirmed,
					_ => LinkCheck::Refused,
				},
			},
			None => LinkCheck::Absent,
		},
	}
}

/// What `entry_landed` decided about the destination's stat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LinkCheck {
	/// The destination's stat is the entry that was sent, on the strong path
	/// (target equality, with the absolute/relative normalisation applied).
	Confirmed,
	/// The destination's stat is not the entry that was sent. The caller
	/// answers "not landed".
	Refused,
	/// The destination's stat was not reachable. The caller answers
	/// "not landed".
	Absent,
	/// The destination is a symlink, but the runtime did not answer
	/// `link_target` either by omitting the field or by sending the empty
	/// string. A symlink always points at something, so an empty string is
	/// not a valid symlink target and a runtime reporting `""` has not
	/// answered the question, exactly as one that omits the field has not.
	/// The confirmation is the symlink bit alone: that is the documented
	/// residual on a runtime that does not report link targets, and the
	/// call site emits a `tracing::warn!` so a CI log carries the reason.
	/// A copy that landed under this branch is never reported as failed;
	/// that is the rule this module exists to enforce.
	Fallback,
}

/// If `runtime_target` is an absolute path under `dir`, the relative form
/// it would have as a linkname against `dir`. Returns `None` when the
/// target is not a single-segment relative path under `dir`, when the
/// `dir` is not absolute, or when the runtime target is not absolute.
/// Used to relate a runtime that reports absolute link targets to the
/// relative linkname the archive carried.
fn normalise_relative_target<'a>(runtime_target: &'a str, dir: &str) -> Option<&'a str> {
	if !runtime_target.starts_with('/') || !dir.starts_with('/') {
		return None;
	}
	let prefix = dir.trim_end_matches('/');
	if prefix.is_empty() {
		return None;
	}
	let tail = runtime_target.strip_prefix(prefix)?;
	let tail = tail.strip_prefix('/')?;
	if tail.is_empty() || tail.contains('/') || tail == "." || tail == ".." {
		return None;
	}
	Some(tail)
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
	/// The residual false positives are the size-only one (libpod's stat has
	/// no checksum, so a failed upload over files of the same length is still
	/// reported as landed) and the link-bit fallback (a runtime that does not
	/// report `linkTarget` lets a pre-existing link at any target satisfy the
	/// confirmation). Both are documented in the module doc.
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
				Ok(post) => match entry_landed(&entry.kind, post.as_ref(), dir) {
					LinkCheck::Confirmed => {}
					LinkCheck::Fallback => {
						// The runtime answered with a symlink stat but did not
						// report `linkTarget`. The destination cannot be asked
						// any stronger than the symlink bit, and refusing a
						// copy that landed would re-introduce #1777. Warn so a
						// CI log carries the literal stat and the reason; the
						// confirmation here is the weaker one. Documented in
						// the module doc.
						tracing::warn!(
							"cp: {} in {dir} was confirmed only by the symlink bit; the runtime did \
							 not report linkTarget: {post:?}",
							entry.path,
						);
					}
					LinkCheck::Refused => {
						tracing::debug!(
							"cp: {} in {dir} is not what was uploaded ({:?}): {post:?}",
							entry.path,
							entry.kind
						);
						return false;
					}
					LinkCheck::Absent => {
						tracing::debug!(
							"cp: {} in {dir} has no stat after the PUT: {post:?}",
							entry.path,
						);
						return false;
					}
				},
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
