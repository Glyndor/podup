//! Unit tests for the watch event filter and the per-path dispatch
//! decision. These cover the two safety properties that have to hold at
//! the notify callback boundary: Access events do not occupy channel
//! slots, and a full bounded channel leaves the overflow flag set when a
//! real event is dropped.

use std::sync::atomic::AtomicBool;

use notify::event::{
	AccessKind, AccessMode, CreateKind, DataChange, ModifyKind, RemoveKind, RenameMode,
};
use notify::EventKind;
use tokio::sync::mpsc;
use tokio::sync::Notify;

use crate::compose::types::{WatchAction, WatchRule};

use super::{enqueue, should_enqueue, sync_op_for, RuleEntry, SyncOp};

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

/// Pin the filter so a refactor that loses the rename family does not
/// silently turn a rename into a no-op. `Modify(Name(..))` covers the
/// three rename shapes the inotify backend actually emits; missing one
/// would let a moved file go undispatched and the container would
/// silently drift from the host.
#[test]
fn should_enqueue_keeps_rename_events() {
	assert!(should_enqueue(&ok_event(EventKind::Modify(
		ModifyKind::Name(RenameMode::Both)
	))));
	assert!(should_enqueue(&ok_event(EventKind::Modify(
		ModifyKind::Name(RenameMode::From)
	))));
	assert!(should_enqueue(&ok_event(EventKind::Modify(
		ModifyKind::Name(RenameMode::To)
	))));
}

/// Pin the access filter against the read-shape. The existing
/// `should_enqueue_drops_access_events` covers `Open(Any)` and
/// `Close(Read)`; this one adds `Access(Read)` and `Open(Read)` so
/// dropping a read never becomes a `should_enqueue == true` after a
/// future refactor that returns the AccessKind variant.
#[test]
fn should_enqueue_drops_read_shapes() {
	assert!(!should_enqueue(&ok_event(EventKind::Access(
		AccessKind::Read
	))));
	assert!(!should_enqueue(&ok_event(EventKind::Access(
		AccessKind::Open(AccessMode::Read)
	))));
}

#[test]
fn enqueue_second_real_event_sets_overflow() {
	// Two Create events into a one-slot channel: the first lands, the
	// second gets `Full` and the loop expects the flag to be set so it
	// can resync on its next iteration.
	let (tx, _rx) = mpsc::channel::<super::WatchEvent>(1);
	let flag = AtomicBool::new(false);
	let wake = Notify::new();
	enqueue(
		&tx,
		&flag,
		&wake,
		ok_event(EventKind::Create(CreateKind::File)),
	);
	assert!(
		!flag.load(std::sync::atomic::Ordering::SeqCst),
		"the first Create into an empty channel must not set the overflow flag"
	);
	enqueue(
		&tx,
		&flag,
		&wake,
		ok_event(EventKind::Create(CreateKind::File)),
	);
	assert!(
		flag.load(std::sync::atomic::Ordering::SeqCst),
		"the second Create into a full channel must set the overflow flag"
	);
}

/// A `Rescan` event is the kernel's own inotify queue overflow signal:
/// the loop's `is_dispatch_event` filter would otherwise drop it
/// silently and the recovery path would never know there was a drop.
/// `should_enqueue` must keep it.
#[test]
fn should_enqueue_keeps_rescan() {
	let rescan = Ok(notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan));
	assert!(should_enqueue(&rescan));
}

/// The `Rescan` event must (a) set the overflow flag so the loop runs
/// the recovery path, and (b) land in the channel so the loop wakes
/// up. The first is the bug; the second is the "the loop never noticed"
/// follow-up that a fix that only sets the flag would introduce.
#[tokio::test]
async fn enqueue_rescan_sets_overflow_and_lands_on_the_channel() {
	let (tx, mut rx) = mpsc::channel::<super::WatchEvent>(4);
	let flag = AtomicBool::new(false);
	let wake = Notify::new();
	let rescan = Ok(notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan));
	enqueue(&tx, &flag, &wake, rescan);
	assert!(
		flag.load(std::sync::atomic::Ordering::SeqCst),
		"a Rescan event must set the overflow flag before any try_send"
	);
	let landed = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv())
		.await
		.expect("Rescan event must reach the receiver within the timeout");
	let landed = landed.expect("the channel must still be open");
	let event = landed.expect("the enqueued item must be Ok");
	assert!(
		event.need_rescan(),
		"the received event must still carry the Rescan flag"
	);
}

/// After the second of two Creates goes into a full channel, the
/// overflow wake must carry a permit that fires a fresh `notified()`
/// future, even when no task is waiting on it at the moment of the
/// store. Without the permit the wakeup would be lost to the
/// drain-then-block race.
#[test]
fn enqueue_overflow_wake_retains_permit() {
	let rt = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.expect("test runtime");
	rt.block_on(async {
		let (tx, _rx) = mpsc::channel::<super::WatchEvent>(1);
		let flag = AtomicBool::new(false);
		let wake = Notify::new();
		enqueue(
			&tx,
			&flag,
			&wake,
			ok_event(EventKind::Create(CreateKind::File)),
		);
		enqueue(
			&tx,
			&flag,
			&wake,
			ok_event(EventKind::Create(CreateKind::File)),
		);
		assert!(
			tokio::time::timeout(std::time::Duration::from_millis(50), wake.notified())
				.await
				.is_ok(),
			"the second enqueue into a full channel must leave a wake permit"
		);
	});
}

/// A successful enqueue into an empty channel must not leave a wake
/// permit behind; the flag and the wake both stay quiet on the happy
/// path so a routine event does not wake the loop unnecessarily.
#[test]
fn enqueue_successful_does_not_leave_wake_permit() {
	let rt = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.expect("test runtime");
	rt.block_on(async {
		let (tx, _rx) = mpsc::channel::<super::WatchEvent>(1);
		let flag = AtomicBool::new(false);
		let wake = Notify::new();
		enqueue(
			&tx,
			&flag,
			&wake,
			ok_event(EventKind::Create(CreateKind::File)),
		);
		assert!(
			tokio::time::timeout(std::time::Duration::from_millis(50), wake.notified())
				.await
				.is_err(),
			"a successful enqueue into an empty channel must not leave a wake permit"
		);
	});
}

// --- rules_not_recovered ------------------------------------------------

/// Build a `RuleEntry` with the fields the tests care about (`service_name`,
/// `rule.path`, `rule.action`, `rule.initial_sync`, `sync_redundant`) and
/// the rest set to plain defaults. `WatchRule` itself is constructed via
/// `serde_yaml` so the test does not have to spell every field out.
fn make_entry(service: &str, path: &str, action: WatchAction, initial_sync: bool) -> RuleEntry {
	let yaml = format!(
		"path: {path}\naction: {}\ninitial_sync: {initial_sync}\n",
		action.as_token(),
	);
	let rule: WatchRule = serde_yaml::from_str(&yaml).expect("WatchRule parses");
	RuleEntry {
		service_name: service.to_string(),
		container_name: format!("{service}-1"),
		rule,
		abs_path: std::path::PathBuf::from(path),
		build_context_abs: None,
		build_context_patterns: Vec::new(),
		sync_redundant: false,
	}
}

/// Plain sync + `initial_sync` is the one case the resync covers: not in
/// the unrecovered list. Plain sync without `initial_sync` is not
/// covered: in the list. `sync+restart` with `initial_sync` is not
/// covered (the restart was not re-run): in the list. A rebuild rule
/// is not covered at all: in the list.
#[test]
fn rules_not_recovered_lists_only_what_recovery_did_not_run() {
	let rules = vec![
		make_entry("web", "src", WatchAction::Sync, true),
		make_entry("web", "lazy", WatchAction::Sync, false),
		make_entry("web", "with_restart", WatchAction::SyncAndRestart, true),
		make_entry("worker", "Dockerfile", WatchAction::Rebuild, true),
	];
	let mut got = super::super::Engine::rules_not_recovered(&rules, &[]);
	got.sort();
	assert_eq!(
		got,
		vec![
			"web:lazy".to_string(),
			"web:with_restart".to_string(),
			"worker:Dockerfile".to_string(),
		]
	);
	// And the recovered rule is not there.
	assert!(!got.iter().any(|s| s == "web:src"));
}

/// A `sync+exec` rule with `initial_sync: true` whose `sync_redundant`
/// flag is set never has its sync step run inside `sync_all`: the rule is
/// dropped before any upload is attempted. From the recovery summary's
/// perspective it is still not a "plain sync + initial_sync" rule, so it
/// is named in the list the way any other non-recovered rule is. A plain
/// sync rule with the same flags cannot exist: the watch loop drops the
/// rule entirely on a self-feeding bind, it does not carry it forward as a
/// `sync_redundant` entry, so this assertion uses `sync+exec` as the
/// stand-in.
#[test]
fn rules_not_recovered_includes_sync_redundant_sync_plus_exec() {
	let mut entry = make_entry("web", "exec_rule", WatchAction::SyncAndExec, true);
	entry.sync_redundant = true;
	let got = super::super::Engine::rules_not_recovered(&[entry], &[]);
	assert_eq!(got, vec!["web:exec_rule".to_string()]);
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
	let wake = Notify::new();
	enqueue(
		&tx,
		&flag,
		&wake,
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
	assert_eq!(sync_op_for(&path).unwrap(), SyncOp::Upload);
}

#[test]
fn sync_op_for_existing_directory_is_upload() {
	let dir = tempfile::tempdir().unwrap();
	let sub = dir.path().join("subdir");
	std::fs::create_dir(&sub).unwrap();
	assert_eq!(sync_op_for(&sub).unwrap(), SyncOp::Upload);
}

#[test]
fn sync_op_for_missing_path_is_remove() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("absent.txt");
	assert!(!path.exists());
	assert_eq!(sync_op_for(&path).unwrap(), SyncOp::Remove);
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
	assert_eq!(sync_op_for(&link).unwrap(), SyncOp::Upload);
}

/// A path under a regular file does not exist: the parent is not a directory,
/// so the kernel returns `ENOTDIR` rather than `ENOENT`. The decision has
/// to land on `Ok(Remove)` for that case so a stale notification (a write to
/// the rule's file that has since moved) does not try to upload a path the
/// filesystem never had.
#[cfg(unix)]
#[test]
fn sync_op_for_path_under_a_file_is_remove() {
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("f.txt");
	std::fs::write(&file, b"x").unwrap();
	let child = file.join("child");
	assert_eq!(sync_op_for(&child).unwrap(), SyncOp::Remove);
}

/// Permission denied on the parent directory: the file is still on disk,
/// but a non-`NotFound` error must propagate, not be turned into a
/// `Remove`. A test that returns `Ok(Remove)` here would happily `rm -f`
/// the container copy on every EACCES.
#[cfg(unix)]
#[test]
fn sync_op_for_permission_error_is_not_remove() {
	use std::os::unix::fs::PermissionsExt;

	let dir = tempfile::tempdir().unwrap();
	let sub = dir.path().join("sub");
	std::fs::create_dir(&sub).unwrap();
	let file = sub.join("f.txt");
	std::fs::write(&file, b"keep").unwrap();

	// Lock the parent so `metadata` on the file fails with `EACCES`.
	let original = std::fs::metadata(&sub).unwrap().permissions();
	std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();

	// Root bypasses the directory mode entirely (DAC override), so the
	// permission check never fires for it; the test then cannot tell the
	// two implementations apart. Detect that shape and bail out with a
	// note rather than falsely reporting a failure that the process is
	// structurally incapable of producing.
	let still_visible = std::fs::metadata(&file).is_ok();
	let outcome = sync_op_for(&file);

	// Restore before any assertion so a panic in the body does not leave
	// the directory unwritable on the way out (Drop would still try to
	// clean it up, and 0o000 makes that fail).
	std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
	let _ = original;

	if still_visible {
		// Root (or any process with `CAP_DAC_READ_SEARCH` / `CAP_DAC_OVERRIDE`).
		// The behaviour we wanted to verify cannot be observed here, so
		// return early instead of declaring a false pass.
		eprintln!("sync_op_for_permission_error_is_not_remove: skipped (process bypasses the directory mode)");
		return;
	}

	assert!(
		outcome.is_err(),
		"a non-NotFound stat error must propagate, not become Ok(Remove); got {outcome:?}"
	);
}

/// A compound rule whose resync upload failed is reported by `sync_all` and
/// is also uncovered by action; it must still be named only once.
#[test]
fn rules_not_recovered_names_a_failed_compound_rule_once() {
	let entry = make_entry("web", "src", WatchAction::SyncAndRestart, true);
	let got = super::super::Engine::rules_not_recovered(&[entry], &["web:src".to_string()]);
	assert_eq!(got, vec!["web:src".to_string()]);
}
