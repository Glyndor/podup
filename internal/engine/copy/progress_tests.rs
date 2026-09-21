//! Tests for the byte counter that backs the `cp` progress row.
//!
//! What we can verify in a unit test: the counter advances as bytes
//! flow, a counter no one reads still produces the correct final
//! count when the operation finishes, and the emitter's verb format
//! carries the byte figure. The full pipeline test (an `Engine::cp`
//! call that drives the counter through `ChannelReader::read` or the
//! PUT body wrapper) lives in `cp_progress_tests.rs`, the same way
//! `pull`'s parser test lives next to the streaming `pull` test.

use std::time::Duration;

use super::{ByteCounter, Emitter};

#[test]
fn a_counter_starts_at_zero() {
	let counter = ByteCounter::new();
	assert_eq!(counter.load(), 0);
}

#[test]
fn add_accumulates() {
	let counter = ByteCounter::new();
	counter.add(1024);
	counter.add(2048);
	counter.add(7);
	assert_eq!(counter.load(), 3079);
}

#[test]
fn add_is_send_so_a_blocking_producer_can_update_it() {
	// The container->host producer side is `ChannelReader::read`,
	// which runs on a `spawn_blocking` thread. The counter must be
	// safe to update from there.
	let counter = ByteCounter::new();
	let counter2 = counter.clone();
	let handle = std::thread::spawn(move || {
		for chunk in [1u64, 2, 3, 4, 5] {
			counter2.add(chunk);
		}
	});
	handle.join().expect("join");
	assert_eq!(counter.load(), 15);
}

/// A counter no one is reading still updates: the container->host
/// path can leave its emitter stopped (or never started) without
/// breaking the counter itself, the same way an interrupted upload
/// that aborts the emitter leaves the final `Copied` line carrying
/// the correct byte count.
#[test]
fn an_unread_counter_still_holds_the_final_total() {
	let counter = ByteCounter::new();
	for chunk in [4096u64, 8192, 12288] {
		counter.add(chunk);
	}
	let total = counter.load();
	let dropped = Emitter {
		handle: tokio::runtime::Builder::new_current_thread()
			.build()
			.unwrap()
			.spawn(async move {}),
	};
	drop(dropped);
	assert_eq!(total, 24576);
}

/// `inner()` is the escape hatch the libpod client's PUT helper
/// needs, so the byte counter can be created outside the cp module
/// (`watch` sync, tests) and still hand its `Arc<AtomicU64>` to the
/// shared `put_bytes_ok_counting` path.
#[test]
fn inner_exposes_the_shared_counter() {
	use std::sync::atomic::AtomicU64;
	let counter = ByteCounter::new();
	let arc: &std::sync::Arc<AtomicU64> = counter.inner();
	counter.add(100);
	assert_eq!(arc.load(std::sync::atomic::Ordering::Relaxed), 100);
}

/// Pin what the emitter puts on the wire. The byte counter is what
/// backs the row's progress, but the verb the operator reads is what
/// it would tell them a copy is doing (#1845). A regression that drops
/// the byte figure from `spawn_emitter` (the `format!("Copying {}",
/// ...)` -> `String::from("Copying")` shape) leaves the row as a
/// spinner over the destination, and the counter-only tests above do
/// not catch it: the counter still advances, the closing verb still
/// reads `Copied <bytes>` via `format_copied_verb`, and the operator
/// loses the only signal that distinguishes a slow copy from a hung
/// one.
///
/// Drives the emitter directly against a [`Capture`], seeded with a
/// non-zero counter so the first tick has a byte figure to render,
/// waits through one interval, and checks the captured verbs. The
/// `current_thread` runtime keeps the emitter task and the test on
/// the same thread, so the [`Capture`]'s thread-local recording flag
/// is visible to the emitter.
#[tokio::test]
async fn the_emitter_verb_carries_the_byte_count() {
	use crate::ui::progress::capture::Capture;
	use crate::ui::progress::Kind;

	let counter = ByteCounter::new();
	// Seed the counter before the first tick so the captured verb has a
	// non-trivial byte figure, not the `0B` of a fresh counter that has
	// not yet seen any work.
	counter.add(2048);
	let capture = Capture::start();
	let emitter = super::spawn_emitter("Cp", "dst".to_string(), counter.clone());
	// `tokio::time::interval`'s first tick fires immediately on the first
	// `.tick().await`, but the task still has to be polled by the runtime.
	// The current_thread runtime parks between awaits; 150 ms is two ticks
	// past the first, which is the narrowest window that reliably observes
	// one.
	tokio::time::sleep(Duration::from_millis(150)).await;
	emitter.stop();

	let verbs: Vec<String> = capture
		.verbs()
		.into_iter()
		.filter(|(k, _, _)| *k == Kind::Cp)
		.map(|(_, _, verb)| verb)
		.collect();
	assert!(
		verbs
			.iter()
			.any(|v| v.starts_with("Copying ") && v.contains("KiB")),
		"the emitter renders `Copying 2.00KiB` with the byte figure; \
		 a bare `Copying` means the byte count was dropped from the \
		 verb: {verbs:?}"
	);
}

/// The closing verb is what stays in the log after a piped `cp`. The
/// counter is not the operator-facing artefact: the verb is. Pin the
/// shape here so a regression that dropped the byte figure from
/// `format_copied_verb` (the `format!("Copied {}", ...)` -> `String::from("Copied")`
/// shape) is caught even when the emitter is short-circuited by a
/// fast copy.
#[test]
fn format_copied_verb_carries_the_byte_count() {
	assert_eq!(super::format_copied_verb(0), "Copied 0B");
	assert_eq!(super::format_copied_verb(13), "Copied 13B");
	// 2048 bytes rounds to 2.0 KiB at one decimal; the formatter trims the
	// trailing zero, so the verb reads `Copied 2.0KiB` rather than
	// `Copied 2.00KiB`. The trailing-zero trim lives in `units::bytes`, and a
	// change to it would surface here too.
	assert_eq!(super::format_copied_verb(2048), "Copied 2.0KiB");
}
