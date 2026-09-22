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
//! `build` already use. On a redirected stderr the start call goes
//! to the plain sink, which buffers the latest transitional verb and
//! prints it once at `progress::end`, so a piped `cp` does not see a
//! frame per update.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::units::{format_bytes, SizeFormat};

/// How often the emitter task reads the counter and rewrites the verb.
/// Matches the live region's repaint cadence so a copy that lasts less
/// than one tick still shows the final byte count before it lands.
const EMIT_INTERVAL: Duration = Duration::from_millis(100);

/// A shared, monotonically-rising byte counter. Producers (the reader
/// in `extract_streamed`, the body wrapper in `put_bytes_ok_counting`)
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

	/// The current byte count. Called from the emitter task.
	pub(crate) fn load(&self) -> u64 {
		self.0.load(Ordering::Relaxed)
	}

	/// The raw shared counter, for callers that need to hand it to a
	/// type outside the module (the libpod client's PUT helper takes
	/// `Arc<AtomicU64>` directly to keep the byte-counting dep out of
	/// the client API).
	pub(crate) fn inner(&self) -> &Arc<AtomicU64> {
		&self.0
	}
}

/// The handle to a running progress emitter. Drop it (or call
/// [`Emitter::stop`]) to abort the task; the spawn point owns it
/// alongside the operation it is reporting on.
pub(super) struct Emitter {
	handle: JoinHandle<()>,
}

impl Emitter {
	/// Abort the emitter. The producer may finish on its own and call
	/// this anyway; aborting an already-finished task is a no-op.
	pub(super) fn stop(self) {
		self.handle.abort();
	}
}

/// Start a task that rewrites the `cp` row's verb from `counter` every
/// [`EMIT_INTERVAL`]. The verb format is `Copying X` where `X` is the
/// byte count rendered with [`format_bytes`] in the same shape `stats`
/// uses (one decimal, binary ladder).
///
/// `kind` and `name` identify the row on the live board. The emitter
/// routes through `progress::start`, so a stderr that is not a tty
/// sees only the most recent transitional verb when the row closes
/// (the plain sink's buffering rule), and a stderr that is a tty sees
/// the verb repainted in place at every tick. A stderr that is not
/// even progress-enabled is the same path the rest of the board
/// takes: a no-op, the counter still advances but no verb fires.
pub(super) fn spawn_emitter(kind: &'static str, name: String, counter: ByteCounter) -> Emitter {
	let handle = tokio::spawn(async move {
		let fmt = SizeFormat::binary().with_decimals(1);
		let mut ticker = tokio::time::interval(EMIT_INTERVAL);
		loop {
			ticker.tick().await;
			let bytes = counter.load();
			let verb = format!("Copying {}", format_bytes(bytes, &fmt));
			crate::ui::progress::start(kind, &name, &verb);
		}
	});
	Emitter { handle }
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
