//! Tests for the streaming packer that the post-PUT confirmation reads from.
//!
//! The streamed bytes are gone after the PUT, so the verification compares the
//! runtime's stat against what the packer recorded while writing the tar. The
//! whole point of moving to a stream (#1844) is that the recorded list stays
//! the only source of "what was uploaded", and the only way to make sure of
//! that is to assert it equals what `sent_entries` would have read back from
//! the same tree packed the same way. Every test in this file pins one shape
//! of that parity so a drift on either side fails here, not at a 220 MB copy
//! in production.
//!
//! The streaming transport calls into the same `pack_path` the archive tests
//! use; the parity test
//! ([`parity::the_recorder_agrees_with_what_sent_entries_reads_back`]) is now
//! also expressed as a single-call assertion: pack to a `Vec` with the
//! recorder on, then assert `recorded == sent_entries(&vec)`.
//!
//! The suite was a single `transport_tests.rs` over the 300-line soft limit,
//! so it became a folder: this module keeps the shared helpers and the
//! index, and each leaf holds the scenarios of one subject.

use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use super::{conclude, pack_path_stream, PackedStream};
use crate::engine::copy::pack::archive::pack_path;
use crate::engine::copy::verify::{sent_entries, SentEntry, SentKind};

/// A fresh counter for the streaming packer. The number never matters — the
/// packer only reads it to advance the `cp` progress row.
fn counter() -> Arc<AtomicU64> {
	Arc::new(AtomicU64::new(0))
}

/// Drive `pack_path_stream` to completion, draining the body stream so the
/// producer is no longer blocked on the bounded channel. Returns the recorded
/// list, or the producer's error (the same error `put_archive_verified` would
/// surface to the caller).
async fn run(packed: PackedStream) -> (Vec<SentEntry>, crate::error::Result<()>) {
	let PackedStream {
		mut body, producer, ..
	} = packed;
	// Drain the body so the producer can finish writing; with no consumer the
	// blocking pack task stalls on `ChannelWriter::send_pending`.
	use futures_util::StreamExt;
	while let Some(item) = body.next().await {
		// A mid-pack error arrives as an `Err` item here. The producer's
		// `pack_outcome` is the path that carries it to the caller; the
		// body-level error is enough to unblock the producer, so we discard it
		// here and check the outcome below.
		let _ = item;
	}
	let outcome = match producer.await {
		Ok(o) => o,
		Err(e) => {
			return (
				Vec::new(),
				Err(crate::error::ComposeError::Build(e.to_string())),
			)
		}
	};
	let (sent, result) = outcome;
	(sent, result)
}

mod closing;
mod parity;
#[cfg(unix)]
mod recording;
#[cfg(unix)]
mod refusals;
