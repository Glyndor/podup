//! The byte counter that backs the `cp` progress row.
//!
//! Both `cp` directions move a measurable amount of data: the
//! container->host stream is a tar archive of unknown size at the
//! start, the host->container PUT is a tar archive whose size is known
//! before the request goes out. In both cases the row verb should
//! advance as work does, the same way `pull`'s `Pulling N/N` verb
//! advances as the layer count grows.
//!
//! The shared counter is the only thing the producer side needs to
//! know about: a `ChannelReader` (stream) or a body wrapper (PUT)
//! adds bytes to it as they flow. The consumer side is a small tokio
//! task that reads the counter on a 100 ms cadence and rewrites the
//! row verb through the same `progress::start` path `up`, `pull` and
//! `build` already use. On a redirected stderr the plain sink prints
//! each transitional verb as a separate line.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::units::{format_bytes, SizeFormat};

/// How often the emitter task reads the counter and rewrites the verb.
/// Matches the live region's repaint cadence. [`Emitter::stop`] writes
/// `Copying <bytes>` only if the task has not written a verb.
const EMIT_INTERVAL: Duration = Duration::from_millis(100);

/// A shared, monotonically-rising byte counter. Producers (the reader
/// in `extract_streamed`, the body wrapper in `receiver_body_with_counter`)
/// `fetch_add` as bytes flow; the emitter task reads and rewrites the
/// row verb.
#[derive(Clone, Debug)]
pub(crate) struct ByteCounter(Arc<AtomicU64>);

impl ByteCounter {
	/// A fresh counter, starting at zero.
	pub(crate) fn new() -> Self {
		Self(Arc::new(AtomicU64::new(0)))
	}

	/// Add `bytes` to the counter. Called from the producer side as
	/// bytes flow through.
	pub(crate) fn add(&self, bytes: u64) {
		self.0.fetch_add(bytes, Ordering::Relaxed);
	}

	/// The current byte count. Read by the emitter task and on stop.
	pub(crate) fn load(&self) -> u64 {
		self.0.load(Ordering::Relaxed)
	}

	/// The raw shared counter, for a producer outside this module that
	/// advances it itself (the upload body built in `pack.rs` takes the
	/// `Arc<AtomicU64>` and adds each frame as it leaves).
	pub(crate) fn inner(&self) -> &Arc<AtomicU64> {
		&self.0
	}
}

/// A running progress emitter and whether its task has written a verb.
/// The operation's owner calls [`Emitter::stop`] before closing the row
/// to record `Copying <bytes>` only if the task has not written a verb,
/// covering copies that finish before the task is ever polled.
/// Dropping the handle does not write a final update.
pub(super) struct Emitter {
	handle: JoinHandle<()>,
	kind: &'static str,
	name: String,
	counter: ByteCounter,
	has_emitted: Arc<AtomicBool>,
}

impl Emitter {
	/// Abort the task, then write `Copying <bytes>` synchronously only if
	/// the task has not written a verb. Call after the producer finishes
	/// and before closing the row to cover a task that was never polled.
	pub(super) fn stop(self) {
		self.handle.abort();
		if !self.has_emitted.load(Ordering::Relaxed) {
			let verb = format_copying_verb(self.counter.load());
			crate::ui::progress::start(self.kind, &self.name, &verb);
		}
	}
}

/// Start a task that rewrites the `cp` row's verb from `counter` every
/// [`EMIT_INTERVAL`]. The verb format is `Copying X` where `X` is the
/// byte count rendered with [`format_bytes`] in the same shape `stats`
/// uses (one decimal, binary ladder).
///
/// `kind` and `name` identify the row on the live board. The emitter
/// routes through `progress::start`, so a stderr that is not a tty
/// sees each transitional verb on its own line, and a stderr that is
/// a tty sees the verb repainted in place at every tick. A stderr that is not
/// even progress-enabled is the same path the rest of the board
/// takes: a no-op, the counter still advances but no verb fires.
pub(super) fn spawn_emitter(kind: &'static str, name: String, counter: ByteCounter) -> Emitter {
	let has_emitted = Arc::new(AtomicBool::new(false));
	let task_name = name.clone();
	let task_counter = counter.clone();
	let task_has_emitted = Arc::clone(&has_emitted);
	let handle = tokio::spawn(async move {
		let mut ticker = tokio::time::interval(EMIT_INTERVAL);
		loop {
			ticker.tick().await;
			let verb = format_copying_verb(task_counter.load());
			crate::ui::progress::start(kind, &task_name, &verb);
			task_has_emitted.store(true, Ordering::Relaxed);
		}
	});
	Emitter {
		handle,
		kind,
		name,
		counter,
		has_emitted,
	}
}

fn format_copying_verb(bytes: u64) -> String {
	let fmt = SizeFormat::binary().with_decimals(1);
	format!("Copying {}", format_bytes(bytes, &fmt))
}

/// Render a static final-verb string carrying the byte count. Used by
/// the call site that owns the operation when it closes the row, so
/// the closing line carries the size and not just `Copied`.
pub(super) fn format_copied_verb(bytes: u64) -> String {
	let fmt = SizeFormat::binary().with_decimals(1);
	format!("Copied {}", format_bytes(bytes, &fmt))
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
