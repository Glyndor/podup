//! Streaming transport for the `cp` and watch-sync packers.
//!
//! The packers themselves live next to where they are called from:
//! [`super::archive_pack::pack_path`] for `cp`, and
//! [`crate::engine::watch::sync::build_sync_tar`] for the watch sync. Both
//! take a writer and a recorder; this module is the writer and the join of
//! the producer task that drives the writer from a bounded channel.
//!
//! Memory is bounded by `CHUNK_BYTES * CHANNEL_CAP`, independent of source
//! size: a 220 MB copy holds roughly that much plus the entry list and one
//! chunk the writer is filling (#1844).

use std::io::{self, Write};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use futures_util::Stream;
use hyper::body::Frame;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::error::ComposeError;

use super::archive_pack::pack_path;
use super::verify::SentEntry;

/// Frames handed to the body stream: a tar chunk, or the terminal error that
/// aborts the upload. The same shape the build-context stream uses so a single
/// client helper accepts both.
pub(super) type BodyItem = io::Result<Frame<Bytes>>;

/// How many pending chunks the channel buffers. Bounds peak memory to about
/// `CHUNK_BYTES * CHANNEL_CAP` regardless of source size, while still letting
/// the blocking tar writer and the async socket writer run concurrently.
pub(super) const CHANNEL_CAP: usize = 8;

/// Coalesce the tar writer's many small writes into ~64 KiB frames, so the
/// body is a handful of sizeable chunks rather than thousands of tiny ones.
pub(super) const CHUNK_BYTES: usize = 64 * 1024;

/// A [`Write`] sink that forwards the tar bytes to an async channel as `Bytes`
/// frames, coalescing small writes to `CHUNK_BYTES`. Blocks (backpressure)
/// when the consumer is behind; errors if the consumer has gone away.
struct ChannelWriter {
	tx: mpsc::Sender<BodyItem>,
	buf: BytesMut,
}

impl ChannelWriter {
	fn send_pending(&mut self) -> io::Result<()> {
		if self.buf.is_empty() {
			return Ok(());
		}
		let chunk = self.buf.split().freeze();
		self.tx.blocking_send(Ok(Frame::data(chunk))).map_err(|_| {
			io::Error::new(
				io::ErrorKind::BrokenPipe,
				"cp: archive receiver dropped before the tar finished",
			)
		})
	}
}

impl Write for ChannelWriter {
	fn write(&mut self, data: &[u8]) -> io::Result<usize> {
		self.buf.extend_from_slice(data);
		if self.buf.len() >= CHUNK_BYTES {
			self.send_pending()?;
		}
		Ok(data.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		self.send_pending()
	}
}

/// The producer side of an in-flight streaming upload: a body stream the PUT
/// request hands to the libpod client, and a join handle whose [`Output`] is
/// the packing outcome (entries + result). The caller awaits the handle
/// after the PUT to learn both what was sent and whether packing errored
/// mid-stream.
///
/// The body stream is `Pin<Box<dyn Stream>>` so callers do not need to
/// import `Pin` themselves; the producer handle is the only place the
/// blocking tar work lives.
pub(in crate::engine) struct PackedStream {
	/// A stream of body frames the client sends as the PUT body. Yields
	/// `io::Result<Frame<Bytes>>` so a mid-pack error reaches hyper as a body
	/// write error rather than truncating the archive silently. The stream
	/// advances `counter` as it yields, so the `cp` progress row keeps
	/// ticking even though the bytes are no longer buffered in `podup`.
	pub(super) body: Pin<Box<dyn Stream<Item = BodyItem> + Send + 'static>>,
	/// The blocking pack task. Its output is `(Vec<SentEntry>, Result<()>)`:
	/// the entries the packer recorded, and the error that should reach the
	/// caller if packing fails (a file vanished, a permission denied), not
	/// the generic "could not be confirmed".
	pub(super) producer: JoinHandle<PackOutcome>,
}

/// What the blocking pack task yields. The `Vec<SentEntry>` is the list the
/// post-PUT confirmation compares against; the `Result` is the error to
/// surface if packing fails before the PUT body finishes. `JoinHandle`
/// errors (the task was aborted or panicked) are mapped to `ComposeError`
/// at the call site, keeping this type the pure outcome of the work.
pub(super) type PackOutcome = (Vec<SentEntry>, Result<(), ComposeError>);

/// Wrap `rx` into a stream that increments `counter` by the size of each
/// yielded frame, so the progress row keeps advancing as bytes leave the
/// body, not as the producer queues them.
fn receiver_body_with_counter(
	rx: mpsc::Receiver<BodyItem>,
	counter: Arc<AtomicU64>,
) -> Pin<Box<dyn Stream<Item = BodyItem> + Send + 'static>> {
	Box::pin(futures_util::stream::unfold(rx, move |mut rx| {
		let counter = counter.clone();
		async move {
			rx.recv().await.map(|item| {
				if let Ok(ref frame) = item {
					if let Some(data) = frame.data_ref() {
						counter.fetch_add(data.len() as u64, Ordering::Relaxed);
					}
				}
				(item, rx)
			})
		}
	}))
}

/// Pack a host file or directory into a streaming archive and record every
/// entry the tar writer appends. The body stream reads from a bounded
/// channel; the blocking pack task does the filesystem walk, the tar
/// assembly, and the per-entry recording on a `spawn_blocking` thread.
///
/// The actual packing (walking the source, classifying entries, recording
/// each one) lives in [`super::archive_pack::pack_path`]. This function is
/// just the transport: a [`ChannelWriter`] behind a [`tar::Builder`], a
/// bounded channel, and the producer task that ties them together.
///
/// `src`, `follow_link`, `name_override` and `contents` carry the same meaning
/// they do for [`super::archive_pack::pack_path`]. `counter` is the shared
/// byte counter the body stream advances as it yields frames, so the `cp`
/// progress row keeps ticking.
///
/// The body stream closes when the producer finishes (or aborts); hyper sees
/// a clean body end and reports `Ok` if the runtime accepted the partial
/// archive, which the verification step then either confirms or refuses.
pub(super) fn pack_path_stream(
	src: &Path,
	follow_link: bool,
	name_override: Option<&str>,
	contents: bool,
	counter: Arc<AtomicU64>,
) -> PackedStream {
	let src = src.to_path_buf();
	// Owned so the closure is `'static`: `tokio::task::spawn_blocking`
	// requires the closure to outlive any borrow of the caller's stack.
	let name_override: Option<String> = name_override.map(str::to_owned);
	let (tx, rx) = mpsc::channel::<BodyItem>(CHANNEL_CAP);

	let producer = tokio::task::spawn_blocking(move || -> PackOutcome {
		let mut writer = ChannelWriter {
			tx,
			buf: BytesMut::with_capacity(CHUNK_BYTES),
		};
		let mut tar = crate::engine::tar_stream::builder(&mut writer);
		tar.follow_symlinks(follow_link);
		let mut sent: Vec<SentEntry> = Vec::new();

		let result = pack_path(
			&src,
			follow_link,
			name_override.as_deref(),
			contents,
			&mut tar,
			&mut sent,
		);

		// Close the tar (writes the two zero-block EOF marker) and flush the
		// channel writer. `into_inner` returns the inner writer; the only
		// way for it to fail is if the channel send fails, which means the
		// consumer (the PUT body) has gone away, which surfaces as a
		// different error anyway.
		if let Err(e) = tar.into_inner() {
			return (sent, Err(cp_err(format!("tar finish: {e}"))));
		}
		if let Err(e) = writer.flush() {
			return (sent, Err(cp_err(format!("flush: {e}"))));
		}

		(sent, result)
	});

	let body = receiver_body_with_counter(rx, counter);

	PackedStream { body, producer }
}

/// As [`pack_path_stream`], but for the watch sync: the body is gzip-wrapped
/// (libpod's archive endpoint accepts either, but the existing sync code
/// gzipped), the entry list is recorded by the same [`super::pack_common`]
/// helpers the `cp` packer uses, and the error mapping keeps the `watch`
/// category instead of `cp`.
pub(in crate::engine) fn build_sync_tar_stream(
	src: &Path,
	entry_name: &Path,
	counter: Arc<AtomicU64>,
) -> PackedStream {
	use flate2::write::GzEncoder;
	use flate2::Compression;

	let src = src.to_path_buf();
	let entry_name = entry_name.to_path_buf();
	let (tx, rx) = mpsc::channel::<BodyItem>(CHANNEL_CAP);

	let producer = tokio::task::spawn_blocking(move || -> PackOutcome {
		let mut channel_writer = ChannelWriter {
			tx,
			buf: BytesMut::with_capacity(CHUNK_BYTES),
		};
		let encoder = GzEncoder::new(&mut channel_writer, Compression::default());
		let mut tar = crate::engine::tar_stream::builder(encoder);
		// Watch sync stores symlinks as links: a symlink inside the watched
		// tree would otherwise copy the contents of its (possibly out-of-tree)
		// target into the container.
		tar.follow_symlinks(false);
		let mut sent: Vec<SentEntry> = Vec::new();

		let result =
			crate::engine::watch::sync::build_sync_tar(&src, &entry_name, &mut tar, &mut sent);

		// `into_inner` runs the gzip footer; an abort along the way surfaces
		// as a watch error so the caller's sync warning line keeps its
		// category.
		if let Err(e) = tar.into_inner() {
			return (sent, Err(watch_err(format!("gzip: {e}"))));
		}
		if let Err(e) = channel_writer.flush() {
			return (sent, Err(watch_err(format!("flush: {e}"))));
		}

		(sent, result)
	});

	let body = receiver_body_with_counter(rx, counter);

	PackedStream { body, producer }
}

/// Map a transport-level error inside the `cp` streaming path to a `cp`
/// `ComposeError`.
fn cp_err(msg: String) -> ComposeError {
	ComposeError::Copy(format!("cp: {msg}"))
}

/// Map a transport-level error inside the watch streaming path to a `watch`
/// `ComposeError`.
fn watch_err(msg: String) -> ComposeError {
	ComposeError::Watch(format!("sync: {msg}"))
}

#[cfg(test)]
#[path = "pack_tests.rs"]
mod tests;
