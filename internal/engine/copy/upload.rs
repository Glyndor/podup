//! PUT a host archive into a container's archive endpoint and confirm it
//! landed, the upload path shared by `cp` and `watch` sync.
//!
//! #1097: on Podman 6 the archive endpoint applies the tar and then closes
//! the connection *without* an HTTP response, which hyper reports as
//! `IncompleteMessage` even though the copy landed (the content does appear,
//! measured on 6.0.1; every raw request to the same endpoint gets a clean
//! 200, so the trigger is client-side and could not be stripped out). To tell
//! that apply-then-close apart from a *genuine* upload failure (a dropped
//! socket, a truncated body), read `dir/entry` after the PUT and treat the
//! copy as landed only if it now **matches what was uploaded**, which is what
//! `uploaded_size` carries.
//!
//! This used to compare the entry's mtime before and after and require it to
//! move. That signal cannot express the question: Podman 6 reports the mtime
//! to whole seconds, so two copies inside one second look identical
//! (#1270: three failures in six back-to-back copies, measured), and
//! re-copying an *unchanged* file is undetectable at any resolution because
//! the extracted file takes the source's own mtime.
//!
//! A source with no single size to compare, a directory above all, is
//! confirmed entry by entry instead (`verify::tree_landed`). Until #1777 it
//! was not confirmed at all, and every directory copy against Podman 6 was
//! reported as failed whether or not it had landed.
//!
//! Fails, rather than guessing, when a post-PUT stat cannot be read or when
//! the archive holds nothing that can be asked about.
//!
//! Inert on Podman 5, which returns a normal response.

use std::pin::Pin;

use futures_util::Stream;

use super::super::Engine;
use super::pack::{BodyItem, PackedStream};
use super::verify::{LandedFailure, SentKind};
use super::{join_archive_path, verify};
use crate::error::{ComposeError, Result};
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;

impl Engine {
	// The two callers (`cp_to_container` in `copy.rs` and the watch sync at
	// `internal/engine/watch/mod.rs`) live in sibling modules of `copy`, so
	// the visibility has to reach `crate::engine` rather than the
	// `copy::super` only.
	#[allow(clippy::too_many_arguments)]
	pub(in crate::engine) async fn put_archive_verified(
		&self,
		container: &str,
		dir: &str,
		entry: &str,
		packed: PackedStream,
		uploaded_kind: Option<SentKind>,
	) -> Result<()> {
		let path = archive_put_path(container, dir);
		let verify_path = (!entry.is_empty()).then(|| {
			format!(
				"{API_PREFIX}/containers/{}/archive?path={}",
				urlencoded(container),
				urlencoded(&join_archive_path(dir, entry)),
			)
		});
		// What the destination entry must look like once the archive is applied.
		//
		// This used to read the entry's mtime *before* the PUT and check that it
		// moved afterwards. That cannot work: Podman 6 reports the mtime to
		// whole seconds, so two copies inside one second are indistinguishable
		// (measured at three failures in six back-to-back copies, #1270), and
		// copying an unchanged file twice is undetectable at any resolution,
		// because the extracted file takes the source's own mtime.
		//
		// The question the confirmation should ask is not "did the entry
		// change" but "does the entry now match what was uploaded". The shape
		// of the match is the same `SentKind` that the tree path uses, so a
		// regular file is checked against a regular file of the same size, a
		// symlink source is checked against a symlink at the destination, and
		// anything else (a directory, a FIFO, an unstatable source) leaves the
		// expectation empty and a later IncompleteMessage asks about every
		// entry of the archive instead.
		let expected = verify_path.as_ref().and(uploaded_kind);

		// `application/gzip` is the honest label for the gzipped tar; Podman
		// sniffs the magic bytes and forgives either. The watch path
		// actually gzips; the cp path uses plain tar, which libpod accepts.
		let PackedStream { body, producer } = packed;
		let body_for_put: Pin<Box<dyn Stream<Item = BodyItem> + Send + 'static>> = body;

		let put_result = self
			.client
			.put_stream_ok(&path, body_for_put, "application/gzip")
			.await;

		// If the PUT itself succeeded with a clean 2xx response (Podman 5),
		// the runtime accepted the bytes and the copy landed. The producer
		// may still be flushing the EOF marker when hyper drops the body
		// stream after the response, in which case `blocking_send` returns
		// `BrokenPipe` ("archive receiver dropped before the tar
		// finished"). That is a timing artifact of the response coming back
		// faster than the producer's final flush; the bytes that matter
		// already went out. Drain the producer best-effort and return Ok,
		// matching Podman 5's "answer is the confirmation" path.
		if put_result.is_ok() {
			let _ = producer.await;
			return Ok(());
		}

		// The PUT errored (IncompleteMessage or something worse). Wait for
		// the pack task to finish and report its outcome: a pack error
		// reaches the caller here, before any verification step runs,
		// because the body stream may have been cut mid-flight and the
		// destination will not match, so the "did not land" message would
		// mask the original cause (a permission denied, a vanished file).
		let sent = match pack_outcome(producer).await {
			Ok(sent) => sent,
			Err(e) => {
				// Pack errored; the PUT body's outcome is no longer useful.
				// Drop the PUT result and surface the pack error.
				drop(put_result);
				return Err(e);
			}
		};

		// Pack succeeded. Now look at the PUT.
		let err = match put_result {
			Ok(()) => return Ok(()),
			Err(e) => e,
		};
		// Only the Podman-6 apply-then-close is recoverable; any other error is a
		// genuine failure and propagates unchanged.
		if !err.is_incomplete_message() {
			return Err(ComposeError::Podman(err));
		}
		let landed: std::result::Result<(), LandedFailure> = match (&verify_path, &expected) {
			(Some(p), Some(want)) => {
				// A symbolic link's destination is read through the 404-with-stat
				// shape, the same dispatch `tree_landed` uses: a dangling link
				// on Podman 5.7.0 returns 404 with the link stat in the header,
				// and `head_path_stat` would throw that stat away. A regular
				// file or directory that answers 404 (the link was cut and the
				// upload failed) returns `None` either way, so the dispatch
				// does not matter for them; links are the only kind that
				// benefit.
				let stat = match want {
					SentKind::Link(_) => self.client.head_path_stat_even_if_missing(p).await,
					_ => self.client.head_path_stat(p).await,
				};
				let abs_entry_path = join_archive_path(dir, entry);
				match stat {
					Ok(post) => {
						match verify::entry_landed(want.clone(), &abs_entry_path, post.as_ref()) {
							verify::LinkCheck::Confirmed => Ok(()),
							verify::LinkCheck::Fallback => {
								tracing::debug!(
									"cp: {entry} in {dir} is a symlink at the destination; \
								 confirmed by type because the runtime sent no linkTarget: \
								 {post:?}"
								);
								Ok(())
							}
							verify::LinkCheck::Refused => {
								tracing::debug!(
									"cp: {entry} in {dir} is not what was uploaded ({want:?}): \
								 {post:?}"
								);
								Err(LandedFailure::Mismatch {
									path: entry.to_string(),
									expected: want.clone(),
									stat: post,
								})
							}
							verify::LinkCheck::Absent => {
								tracing::debug!(
								"cp: {entry} in {dir} could not be read back: no stat in response"
							);
								Err(LandedFailure::Mismatch {
									path: entry.to_string(),
									expected: want.clone(),
									stat: None,
								})
							}
						}
					}
					Err(stat_err) => {
						tracing::debug!(
							"cp: could not re-verify {p} after an incomplete PUT: {stat_err}"
						);
						Err(LandedFailure::StatError {
							path: entry.to_string(),
							error: stat_err.to_string(),
						})
					}
				}
			}
			_ => self.tree_landed(container, dir, sent).await,
		};
		let failure = match landed {
			Ok(()) => return Ok(()),
			Err(failure) => failure,
		};
		// The upload finished but its result could not be confirmed. Say what
		// was wrong with the destination, with the entry and the stat the
		// runtime answered, and keep the actionable hint, instead of surfacing
		// the raw transport error. The hint is true and useful even when the
		// verification was a clear mismatch: bytes that look like the upload
		// can sit in the destination for a moment before the runtime
		// cleans them up.
		let detail = verify::format_landed_failure(&failure, dir);
		Err(ComposeError::Copy(format!(
			"the upload to {dir} could not be confirmed: the container runtime closed the \
			 connection without a response, and {detail}. The copy may or may not have landed; \
			 check {dir} in the container."
		)))
	}
}

/// Build the libpod archive-PUT path. `copyUIDGID=false` overrides the
/// libpod default of `true`, which would otherwise overwrite the host
/// UID/GID on the destination file with the container's runtime
/// UID/GID. The Docker compat handler defaulted `copyUIDGID` to
/// false, so this is the line that keeps podup's user-visible
/// behaviour stable across the switch (#1914).
///
/// Public to `crate::engine` so the wire-shape unit tests in
/// `engine::lifecycle::libpod_endpoint_query_tests` can pin the
/// query string without standing up the streaming packer.
pub(in crate::engine) fn archive_put_path(container: &str, dir: &str) -> String {
	format!(
		"{API_PREFIX}/containers/{}/archive?path={}&copyUIDGID=false",
		urlencoded(container),
		urlencoded(dir),
	)
}

/// Wait for the pack task and return the recorded entry list. A pack error
/// short-circuits here so the verification step never runs against a
/// partially-built list, and the caller sees the original "permission
/// denied / file vanished" message rather than the generic "did not land"
/// message a truncated upload would otherwise produce.
async fn pack_outcome(
	producer: tokio::task::JoinHandle<super::pack::PackOutcome>,
) -> Result<Vec<verify::SentEntry>> {
	let outcome = producer
		.await
		.map_err(|e| ComposeError::Build(format!("cp: archive pack task: {e}")))?;
	let (sent, result) = outcome;
	result?;
	Ok(sent)
}
