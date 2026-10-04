//! Unit tests for the watch event filter and the per-path dispatch
//! decision. These cover the two safety properties that have to hold at
//! the notify callback boundary: Access events do not occupy channel
//! slots, and a full bounded channel leaves the overflow flag set when a
//! real event is dropped.

use std::sync::atomic::AtomicBool;

use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind, RemoveKind};
use notify::EventKind;
use tokio::sync::mpsc;

use super::{enqueue, should_enqueue, sync_op_for, SyncOp};

fn ok_event(kind: EventKind) -> notify::Result<notify::Event> {
	Ok(notify::Event::new(kind))
}

#[test]
fn should_enqueue_keeps_errors_and_dispatch_events() {
	// Errors must always reach the loop: it is what warns on them.
	assert!(should_enqueue(&Err(notify::Error::generic("x"))));
	// Create / Modify / Remove drive a sync; keep those.
	assert!(should_enqueue(&ok_event(EventKind::Create(
		CreateKind::File
	))));
	assert!(should_enqueue(&ok_event(EventKind::Modify(
		ModifyKind::Data(DataChange::Any)
	))));
	assert!(should_enqueue(&ok_event(EventKind::Remove(
		RemoveKind::File
	))));
}

#[test]
fn should_enqueue_drops_access_events() {
	// The whole point of the filter: inotify's `OPEN` would otherwise fill
	// the channel with reads during the initial sync.
	assert!(!should_enqueue(&ok_event(EventKind::Access(
		AccessKind::Open(AccessMode::Any)
	))));
	assert!(!should_enqueue(&ok_event(EventKind::Access(
		AccessKind::Close(AccessMode::Read)
	))));
}

#[test]
fn enqueue_second_real_event_sets_overflow() {
	// Two Create events into a one-slot channel: the first lands, the
	// second gets `Full` and the loop expects the flag to be set so it
	// can resync on its next iteration.
	let (tx, _rx) = mpsc::channel::<super::WatchEvent>(1);
	let flag = AtomicBool::new(false);
	enqueue(&tx, &flag, ok_event(EventKind::Create(CreateKind::File)));
	enqueue(&tx, &flag, ok_event(EventKind::Create(CreateKind::File)));
	assert!(
		flag.load(std::sync::atomic::Ordering::SeqCst),
		"the second Create into a full channel must set the overflow flag"
	);
}

#[test]
fn enqueue_filtered_event_does_not_touch_a_full_channel() {
	// The flag is fresh, the channel is fresh and already holds one
	// event (so it is full), and we enqueue an Access(Open) event. The
	// filter must drop it before any `try_send`: the flag stays false
	// and the channel still holds the one original event.
	let (tx, mut rx) = mpsc::channel::<super::WatchEvent>(1);
	let pre = ok_event(EventKind::Create(CreateKind::File));
	tx.try_send(pre)
		.expect("first Create lands in a 1-slot channel");
	let flag = AtomicBool::new(false);
	enqueue(
		&tx,
		&flag,
		ok_event(EventKind::Access(AccessKind::Open(AccessMode::Any))),
	);
	assert!(
		!flag.load(std::sync::atomic::Ordering::SeqCst),
		"a filtered Access event must not set the overflow flag"
	);
	// Drive the receiver to completion and confirm the channel still
	// holds exactly the one event that was in it before.
	let rt = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.expect("test runtime");
	rt.block_on(async {
		let first = rx.recv().await.expect("the original event is still queued");
		assert!(
			matches!(first, Ok(ref e) if matches!(e.kind, EventKind::Create(CreateKind::File)))
		);
		// `recv` would block forever on an empty channel; race it with a
		// 50 ms timeout and treat the `Timeout` as proof the channel is
		// empty after the one event was consumed.
		assert!(
			tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv())
				.await
				.is_err(),
			"the channel held more than one event after the filtered enqueue"
		);
	});
}

// --- sync_op_for --------------------------------------------------------

#[test]
fn sync_op_for_existing_file_is_upload() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("present.txt");
	std::fs::write(&path, b"present").unwrap();
	assert_eq!(sync_op_for(&path), SyncOp::Upload);
}

#[test]
fn sync_op_for_existing_directory_is_upload() {
	let dir = tempfile::tempdir().unwrap();
	let sub = dir.path().join("subdir");
	std::fs::create_dir(&sub).unwrap();
	assert_eq!(sync_op_for(&sub), SyncOp::Upload);
}

#[test]
fn sync_op_for_missing_path_is_remove() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("absent.txt");
	assert!(!path.exists());
	assert_eq!(sync_op_for(&path), SyncOp::Remove);
}

#[cfg(unix)]
#[test]
fn sync_op_for_dangling_symlink_is_upload() {
	// A dangling symlink is present in `symlink_metadata` terms, so the
	// initial sync's own check accepts it, and the dispatch decision has
	// to agree. If this ever flips to `Remove`, a rule whose target is a
	// link would be wrong on the very first event.
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("link");
	std::os::unix::fs::symlink("/nonexistent/target", &link).unwrap();
	assert_eq!(sync_op_for(&link), SyncOp::Upload);
}
