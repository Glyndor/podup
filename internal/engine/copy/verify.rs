//! Confirming a tree upload that the runtime never answered.
//!
//! A single file is confirmed by its size and its kind at the destination. A
//! directory has no size that says anything about its children, so #1777 had
//! every directory copy against Podman 6 reported as failed, landed or not.
//! The question asked here is the same one, put to each entry: is every
//! regular file of the archive that was sent at the destination with its
//! size, every directory there as a directory, and every symbolic link there
//! as a symlink with the target the archive carried.
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
//!   destination whose `linkTarget` (Podman 5.7.0 carries it in
//!   `X-Docker-Container-Path-Stat`, including on a 404 for a dangling link
//!   that `head_path_stat_even_if_missing` reads) matches the sent target
//!   after the lexical normalization Podman applies. `size` is also compared
//!   against the byte length of the target as written: independent evidence,
//!   and the two together close any one of them being stale. The stat for a
//!   dangling link on Podman 5.7.0 was carried on the 404 response in the
//!   same header, which `head_path_stat` threw away;
//!   `head_path_stat_even_if_missing` reads it back so a cut stream cannot
//!   be confirmed against the link that was already there.
//! - Hard links, FIFOs, char/block devices and anything else: an entry of a
//!   kind that cannot be asked about through the archive stat makes the
//!   archive unverifiable. `sent_entries` returns an error naming the type,
//!   the same way a non-UTF-8 path already does, so the tree answer is "not
//!   landed" and the upload fails closed.
//!
//! ## Why link confirmation got harder
//!
//! `entry_landed` used to satisfy any link on the type check alone
//! (`post.is_some_and(is_symlink)`), which meant an existing symlink at the
//! destination path — whatever it pointed at — confirmed an uploaded symlink.
//! A `cp` whose upload was cut, or that landed on a tree where a link of the
//! same name already pointed somewhere else, was reported as landed.
//!
//! Podman 5.7.0 already sent the target back in the stat header
//! (`linkTarget`, normalised against the directory the link lives in). The
//! sent target was read out of the tar in `sent_entries` and thrown away.
//! `entry_landed` now compares the two, after applying the same lexical
//! normalisation to the sent side: the target Podman reports for a relative
//! link `/tmp/d/rel -> ../etc/hosts` is `/tmp/etc/hosts` (joined to `/tmp/d`,
//! `..` resolved lexically, with the filesystem not touched — the dangling
//! case proves Podman does not resolve it either), so a literal
//! `sent_target == stat.link_target` would refuse every relative link in a
//! real tree and break every directory copy. Normalising the sent side the
//! same way Podman does is the only way both agree.
//!
//! ## Paths
//!
//! Every path is taken from the tar as UTF-8. An entry whose path has a byte
//! that is not valid UTF-8 makes `sent_entries` return an error, which makes
//! `tree_landed` answer "not landed". A copy of such a tree against Podman 6
//! with a dropped response is then reported as failed rather than confirmed
//! against the lossy-rewritten name. The link target is read the same way:
//! an entry whose link name is absent or not valid UTF-8 returns an error
//! from `sent_entries`, so an entry the destination cannot be asked about
//! is not silently confirmed.
//!
//! ## Why the failure carries the entry and the stat
//!
//! `tree_landed` and the single-entry path in `put_archive_verified` used to
//! return `bool`, which left the caller no way to tell which entry failed or
//! what the runtime had said about it. Two rounds of guessing at the Podman 6
//! link confirmation (#1808) were blind for that reason: the failing entry
//! went out through `tracing::debug!`, the integration suite's env filter does
//! not emit debug, and the job log carried no trace of which entry or which
//! stat. The verification now returns a [`LandedFailure`] that names the entry,
//! the kind expected, and the literal `PathStat` the runtime answered, so the
//! next attempt starts from a measurement.

#[cfg(test)]
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
	/// A symbolic link with the target the tar header carries, as written.
	/// A `linkTarget` Podman reports has been normalised against the
	/// directory the link lives in; the comparison in `entry_landed`
	/// applies the same normalisation to this side so the two agree.
	Link(String),
}

/// One entry of the uploaded archive that the destination can be asked about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::engine) struct SentEntry {
	/// Relative to the directory the archive was extracted at, `/`-separated,
	/// without a leading `./` or a trailing `/`.
	pub(super) path: String,
	pub(super) kind: SentKind,
}

/// The regular files, directories and symbolic links in an archive `cp`
/// uploads, in archive order. The archive is plain tar; `cp`'s local-socket
/// path does not gzip (the bytes never leave the host).
///
/// Symbolic links are kept here even though `head_path_stat` cannot ask about
/// them: on Podman 5.7.0 the 404 for a dangling link still carried the
/// `X-Docker-Container-Path-Stat` header, and the runtime is asked through
/// `head_path_stat_even_if_missing` so the stat is read on the 404 too.
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
/// confirmed against a lossy-rewritten name. An entry whose symlink target
/// (the link name in the tar header) is absent or not valid UTF-8 is the
/// same shape: the destination cannot be asked about a target the entry did
/// not name, so the archive is unverifiable and the function returns an
/// error. So is an entry whose path is the extraction directory itself (`.`),
/// which the caller confirmed before uploading.
///
/// **Test-only oracle.** Production verification reads the recorder the
/// packer fills in as it writes the tar; the streamed bytes are gone after
/// the PUT, so `sent_entries` does not run in production. It is gated to
/// test builds (`#[cfg(test)]`) for that reason, and exists for the
/// recorded-equals-written parity test
/// ([`super::pack_tests::the_recorder_agrees_with_what_sent_entries_reads_back`]):
/// pack to a `Vec<u8>` with the recorder on, then assert the recorder
/// equals what `sent_entries` reads back from those same bytes. This pins
/// that the recorder and the archive agree on every entry; a sabotage that
/// records one without writing the other (or vice versa) breaks the test
/// in lock-step.
#[cfg(test)]
pub(super) fn sent_entries(gz_tar: &[u8]) -> Result<Vec<SentEntry>> {
	let mut archive = tar::Archive::new(gz_tar);
	let mut sent = Vec::new();
	for entry in archive.entries().map_err(ComposeError::Io)? {
		let entry = entry.map_err(ComposeError::Io)?;
		let entry_type = entry.header().entry_type();
		let kind = match entry_type {
			tar::EntryType::Regular | tar::EntryType::Continuous => SentKind::File(entry.size()),
			tar::EntryType::Directory => SentKind::Dir,
			tar::EntryType::Symlink => {
				// `link_name_bytes` returns `None` when the tar carries no
				// link name for the entry (a malformed archive). `to_str`
				// rejects bytes that are not valid UTF-8. Both make the
				// entry unverifiable: we cannot compare what the entry did
				// not name against what the destination reports, and
				// answering a question we did not ask would just be the
				// pre-#1808 false positive in another shape.
				let target_bytes = entry.link_name_bytes().ok_or_else(|| {
					ComposeError::Build("cp: archive symlink entry has no link name".into())
				})?;
				let target = std::str::from_utf8(target_bytes.as_ref())
					.map_err(|_| {
						ComposeError::Build(
							"cp: archive symlink entry link name is not UTF-8".into(),
						)
					})?
					.to_string();
				SentKind::Link(target)
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

/// What the destination's stat said about the entry that was sent.
///
/// The four-way result exists so the link confirmation's log can distinguish
/// "confirmed because the target matched" from "confirmed because the
/// runtime did not send a target, so we fell back to the type check we used
/// to live on". The pre-#1808 check satisfied any link on `is_symlink`; a
/// runtime that returns `linkTarget: ""` puts us back on that check, and
/// the operator looking at the log should be able to tell which side of
/// the line they are on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkCheck {
	/// The destination matched what was sent exactly (size, kind, and for
	/// links, the target).
	Confirmed,
	/// The destination did not match what was sent. The caller turns this
	/// into a `LandedFailure::Mismatch`.
	Refused,
	/// No answer was available: the runtime did not return a stat for this
	/// entry at all. For a non-link entry this is the path-was-actually-
	/// missing shape; for a link it is the runtime that drops the stat
	/// header even on a 200. The caller turns this into `Mismatch { stat:
	/// None }` so the user-facing message names the missing stat.
	Absent,
	/// The runtime answered a stat, but without a `linkTarget` to compare
	/// against (an older Podman, or a Podman that stops sending the field
	/// in a future release). The link is still confirmed, because refusing
	/// every link on a missing field would turn a runtime change into a
	/// fleet-wide copy failure. The reason string the caller logs must say
	/// "fallback", so the operator can tell this confirmation from one
	/// backed by a real target match.
	Fallback,
}

/// Whether the destination's stat is the entry that was sent.
///
/// `entry_path` is the absolute container path the stat was taken at — the
/// same string `head_path_stat[_even_if_missing]` was pointed at. It is
/// used to normalise a relative sent target against the directory the link
/// lives in, the same way Podman does before reporting `linkTarget`.
///
/// A regular file must match in size and have no Go `os.ModeType` bit set:
/// a directory, a symlink, a named pipe, a socket, a device or any other
/// non-regular kind reports a size that says nothing about what the upload
/// put there, so the size comparison on its own is not enough. A link is
/// confirmed only when its target text matches and its byte length matches,
/// or the runtime sent no `linkTarget` and the link is at least a link at
/// the destination.
pub(super) fn entry_landed(sent: SentKind, entry_path: &str, post: Option<&PathStat>) -> LinkCheck {
	let is_dir = |stat: &PathStat| stat.mode & (1 << 31) != 0;
	let is_regular = |stat: &PathStat| stat.mode & MODE_TYPE == 0;
	let is_symlink = |stat: &PathStat| stat.mode & (1 << 27) != 0;
	match (sent, post) {
		(SentKind::File(size), Some(stat)) => {
			if is_regular(stat) && stat.size == size {
				LinkCheck::Confirmed
			} else {
				LinkCheck::Refused
			}
		}
		(SentKind::Dir, Some(stat)) => {
			if is_dir(stat) {
				LinkCheck::Confirmed
			} else {
				LinkCheck::Refused
			}
		}
		(SentKind::Link(sent_target), Some(stat)) => {
			if !is_symlink(stat) {
				return LinkCheck::Refused;
			}
			if stat.link_target.is_empty() {
				// Runtime did not send `linkTarget`; fall back to the
				// pre-#1808 type check. The caller logs this as a
				// fallback, not a confirmation by target.
				return LinkCheck::Fallback;
			}
			let normalized = normalize_link_target(&sent_target, entry_path);
			let target_bytes = sent_target.len();
			if stat.link_target == normalized && stat.size as usize == target_bytes {
				LinkCheck::Confirmed
			} else {
				LinkCheck::Refused
			}
		}
		(_, None) => LinkCheck::Absent,
	}
}

/// Bring a sent link target into the shape Podman reports in `linkTarget`.
/// Absolute targets are returned unchanged (already what Podman reports).
/// Relative targets are joined to the directory part of `entry_path` and
/// resolved `.` / `..` lexically, without touching the filesystem: a
/// dangling link points somewhere that does not exist, and Podman resolves
/// the `..` lexically the same way (measured on Podman 5.7.0, 2026-09-20:
/// `/tmp/d/rel -> ../etc/hosts` reports `/tmp/etc/hosts`, which does not
/// exist).
fn normalize_link_target(sent_target: &str, entry_path: &str) -> String {
	if sent_target.starts_with('/') {
		return sent_target.to_string();
	}
	let entry_dir = parent_dir_absolute(entry_path);
	let joined = match entry_dir {
		"" => sent_target.to_string(),
		"/" => format!("/{}", sent_target),
		_ => format!("{}/{}", entry_dir, sent_target),
	};
	lex_normalize(&joined)
}

/// Directory portion of an absolute container path. `/` is its own parent;
/// `/foo` has parent `/`; `/foo/bar` has parent `/foo`. Returns the empty
/// string only when the input has no leading `/`, which a non-root call
/// site should not produce.
fn parent_dir_absolute(path: &str) -> &str {
	if path == "/" {
		return "/";
	}
	match path.rfind('/') {
		Some(0) => "/",
		Some(idx) => &path[..idx],
		None => "",
	}
}

/// Resolve `.` and `..` lexically without touching the filesystem. A
/// leading `..` past the root is kept as a literal `..`, so a path that
/// escapes the root stays a `..`-prefixed path.
///
/// Splits on `/` and nothing else, deliberately, rather than going through
/// `std::path::Path::components`. The path being normalised is a container
/// path that came out of a tar header, and it is POSIX on every host: a
/// backslash in it is an ordinary character in a file name, not a
/// separator. `Path::components` would agree on Unix and disagree on
/// Windows, where it splits on `\\` too, so a link named `a\\b` would
/// normalise to one component on the Linux lane and two on the Windows
/// lane and the verification verdict would depend on the host running
/// `podup`, not on what the runtime reported. Splitting by hand keeps one
/// answer everywhere.
fn lex_normalize(path: &str) -> String {
	let absolute = path.starts_with('/');
	let mut stack: Vec<&str> = Vec::new();
	for part in path.split('/') {
		match part {
			"" | "." => {}
			".." => match stack.last() {
				Some(top) if *top != ".." => {
					stack.pop();
				}
				_ => stack.push(".."),
			},
			name => stack.push(name),
		}
	}
	if stack.is_empty() {
		return if absolute {
			"/".to_string()
		} else {
			".".to_string()
		};
	}
	let joined = stack.join("/");
	if absolute {
		format!("/{joined}")
	} else {
		joined
	}
}

/// Test-only seam: the upload integration tests fake the runtime's stat
/// header (the only way to exercise the link confirmation without a real
/// Podman), and they need the same lexical normalisation
/// `entry_landed` runs on the sent side so the value they plant in the
/// header is what Podman would have reported. Production code does not
/// reach for this; it goes through `entry_landed`.
#[cfg(test)]
pub(crate) fn normalize_for_test(path: &str) -> String {
	lex_normalize(path)
}

/// What the verification saw when it refused the upload. The fields name the
/// entry, what was expected, and what the runtime answered (or that it could
/// not be asked at all, or that the archive itself was unreadable), so the
/// next diagnosis is not blind: the next attempt's log carries the entry that
/// failed and the stat that was read for it, not just a verdict.
///
/// The previous bool return forced the caller to invent a message; that
/// message named neither entry nor stat, and the `tracing::debug!` that did
/// carry them was filtered out of the integration env. This is the type the
/// bool became, so the message reaches the user.
#[derive(Debug)]
pub(crate) enum LandedFailure {
	/// The runtime answered, but the answer is not what was sent. The most
	/// common case: an entry left over from before the upload, an entry the
	/// stream cut before reaching, or an entry the destination cannot host
	/// (a directory where a file was sent, say).
	Mismatch {
		/// Path of the entry that failed, relative to `dir`.
		path: String,
		/// What the upload was supposed to land.
		expected: SentKind,
		/// What the stat endpoint reported. `None` is the 404-without-stat
		/// shape reachable for symlinks on runtimes that drop the
		/// `X-Docker-Container-Path-Stat` header.
		stat: Option<PathStat>,
	},
	/// The runtime could not be asked about this entry at all (a 5xx, a
	/// socket drop on the stat `HEAD` itself).
	StatError { path: String, error: String },
	/// The archive sent could not be read back, or held no verifiable
	/// entries, so no specific entry was named. Carries the reason only.
	Unnamed(String),
}

/// Render `failure` into the inner clause the caller's error wraps. Names
/// the path, what was expected, and what the runtime answered, so a user (or
/// the next diagnosis) can act on it. `dir` is the directory the archive was
/// extracted at, used to qualify the entry path in the message.
///
/// The outer "the upload to {dir} could not be confirmed" prefix lives at
/// the call site; the part this function produces is what was wrong with
/// the destination, why the verdict is what it is.
pub(crate) fn format_landed_failure(failure: &LandedFailure, dir: &str) -> String {
	match failure {
		LandedFailure::Mismatch {
			path,
			expected,
			stat,
		} => {
			let stat_part = match stat {
				Some(s) => format!("runtime answered {s:?}"),
				None => "runtime answered no stat (a 404 without the stat header)".to_string(),
			};
			format!("{path} in {dir} is not what was uploaded; expected {expected:?}, {stat_part}")
		}
		LandedFailure::StatError { path, error } => {
			format!("{path} in {dir} could not be read back after an incomplete PUT: {error}")
		}
		LandedFailure::Unnamed(reason) => {
			format!("the archive that was sent could not be read back: {reason}")
		}
	}
}

impl Engine {
	/// Whether every file, directory and symbolic link the packer recorded
	/// is at `dir` in `container`. On refusal returns a [`LandedFailure`]
	/// that names the entry, the kind expected, and the stat the runtime
	/// answered (or the transport error, or that the archive itself was
	/// unverifiable), so the next diagnosis starts from a measurement.
	///
	/// `sent` is the list the packer assembled while writing the tar
	/// (the bytes are gone after the PUT, so this is the only source of
	/// "what was uploaded" left). Every entry that `sent_entries` would
	/// have refused on the read-back side has already been refused here:
	/// non-UTF-8 paths and link names, hard links, FIFOs, char/block devices.
	/// The same rules apply through [`crate::engine::copy::pack`]'s recorder,
	/// so the two cannot disagree about what the destination can be asked
	/// about.
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
	pub(super) async fn tree_landed(
		&self,
		container: &str,
		dir: &str,
		sent: Vec<SentEntry>,
	) -> std::result::Result<(), LandedFailure> {
		if sent.is_empty() {
			return Err(LandedFailure::Unnamed(
				"the archive held no verifiable entries".into(),
			));
		}
		for entry in sent.iter().rev() {
			let abs_path = join_archive_path(dir, &entry.path);
			let stat_path = format!(
				"{API_PREFIX}/containers/{}/archive?path={}",
				urlencoded(container),
				urlencoded(&abs_path),
			);
			let stat = match &entry.kind {
				SentKind::Link(_) => self.client.head_path_stat_even_if_missing(&stat_path).await,
				_ => self.client.head_path_stat(&stat_path).await,
			};
			match stat {
				Ok(post) => match entry_landed(entry.kind.clone(), &abs_path, post.as_ref()) {
					LinkCheck::Confirmed => {}
					LinkCheck::Fallback => {
						tracing::debug!(
							"cp: {} in {dir} is a symlink at the destination; confirmed by type \
							 because the runtime sent no linkTarget: {post:?}",
							entry.path,
						);
					}
					LinkCheck::Refused => {
						tracing::debug!(
							"cp: {} in {dir} is not what was uploaded ({:?}): {post:?}",
							entry.path,
							entry.kind,
						);
						return Err(LandedFailure::Mismatch {
							path: entry.path.clone(),
							expected: entry.kind.clone(),
							stat: post,
						});
					}
					LinkCheck::Absent => {
						tracing::debug!(
							"cp: {} in {dir} could not be read back: no stat in response",
							entry.path,
						);
						return Err(LandedFailure::Mismatch {
							path: entry.path.clone(),
							expected: entry.kind.clone(),
							stat: None,
						});
					}
				},
				Err(stat_err) => {
					tracing::debug!(
						"cp: could not re-verify {} in {dir} after an incomplete PUT: {stat_err}",
						entry.path
					);
					return Err(LandedFailure::StatError {
						path: entry.path.clone(),
						error: stat_err.to_string(),
					});
				}
			}
		}
		Ok(())
	}
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
