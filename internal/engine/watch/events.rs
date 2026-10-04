//! Watch-event filter and per-path dispatch decision (#1984).
//!
//! [`should_enqueue`] keeps only events that drive a sync, plus errors, so
//! inotify's `Access(Open)` for every file the initial sync reads never takes
//! a slot in the bounded channel. [`enqueue`] records an overflow when the
//! channel is full, so the loop can resync. [`sync_op_for`] decides upload or
//! removal for each path from the host, not from the kind of the first event
//! in a debounce batch.
//! `Engine::sync_all` runs the initial sync, and the full resync after an
//! overflow.

use std::collections::HashSet;
use std::path::Path;

use tokio::sync::mpsc;
use tracing::debug;

use super::placement::is_dispatch_event;
use super::RuleEntry;
use crate::engine::Engine;
use tracing::{info, warn};

/// The mpsc item produced by the notify callback, after the
/// [`should_enqueue`] filter has run. Matches the notify crate's own
/// [`notify::Result`]; the watch loop just unwraps the same shape.
pub(super) type WatchEvent = notify::Result<notify::Event>;

/// True when `res` should occupy a slot in the watch channel.
///
/// Errors must reach the loop so it can `warn!` on them; the rest of the
/// keep set is exactly what `is_dispatch_event` would let through
/// downstream, so the loop's own filter (kept for defence in depth) is a
/// no-op when this gate is in place.
pub(super) fn should_enqueue(res: &WatchEvent) -> bool {
	match res {
		Err(_) => true,
		Ok(e) => is_dispatch_event(&e.kind),
	}
}

/// Push `res` into the watch channel, or record an overflow.
///
/// The non-keep cases are dropped here so the bounded channel can hold
/// only meaningful events; on `TrySendError::Full` the overflow flag is
/// set so the loop resyncs on its next iteration, and the existing
/// `debug!` line keeps the drop observable to operators. `Closed` is the
/// normal "the consumer went away" path and gets only the same `debug!`
/// line.
pub(super) fn enqueue(
	tx: &mpsc::Sender<WatchEvent>,
	overflow: &std::sync::atomic::AtomicBool,
	res: WatchEvent,
) {
	if !should_enqueue(&res) {
		return;
	}
	match tx.try_send(res) {
		Ok(()) => {}
		Err(mpsc::error::TrySendError::Full(res)) => {
			overflow.store(true, std::sync::atomic::Ordering::SeqCst);
			debug!("watch event dropped (channel full): {res:?}");
		}
		Err(mpsc::error::TrySendError::Closed(res)) => {
			debug!("watch event dropped (channel closed): {res:?}");
		}
	}
}

/// What to do with a single host path in a dispatch batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SyncOp {
	/// The path is present on the host: tar it and PUT it into the container.
	Upload,
	/// The path is gone on the host: mirror the deletion into the container.
	Remove,
}

/// Upload when the path exists on the host, remove when it does not.
///
/// `symlink_metadata` does not follow links, so a dangling symlink counts as
/// present and is uploaded as a link, the same rule the initial sync applies.
pub(super) fn sync_op_for(path: &Path) -> SyncOp {
	match std::fs::symlink_metadata(path) {
		Ok(_) => SyncOp::Upload,
		Err(_) => SyncOp::Remove,
	}
}

impl Engine {
	/// Run the per-rule sync loop. Used both at startup (`only_initial =
	/// true` honours the rule's `initial_sync` flag) and on an
	/// overflow-driven resync (`only_initial = false`, every sync rule
	/// is refreshed to recover state the dropped event may have carried).
	///
	/// The startup path keeps the behaviour the original inline loop
	/// had: no action check (any rule with `initial_sync` and a target
	/// is synced). The resync path adds the action check because
	/// `rebuild` / `restart` rules do not sync anything, and only the
	/// sync family would have been affected by a dropped event.
	pub(super) async fn sync_all(
		&self,
		rule_entries: &[RuleEntry],
		ensured: &mut HashSet<(String, String)>,
		only_initial: bool,
	) {
		for entry in rule_entries {
			if only_initial {
				// Startup: keep the original behaviour. The pre-existing
				// loop did not filter by action, so do not add an action
				// check here; the only change is extracting the body.
				if !entry.rule.initial_sync {
					continue;
				}
			} else if !entry.rule.action.requires_target() {
				// Resync: skip `rebuild` and `restart`, which never sync.
				continue;
			}
			// A missing watch path cannot be synced: the watcher-setup
			// loop above will warn about it, so skip both the
			// `initial sync` log and the (doomed) sync itself.
			// `symlink_metadata` is used (not `exists`) so a path that
			// exists only as a dangling symlink is still synced: the
			// packer in `watch/sync.rs` preserves links, and the rule's
			// intent there is to upload the link itself.
			if std::fs::symlink_metadata(&entry.abs_path).is_err() {
				continue;
			}
			let Some(target) = &entry.rule.target else {
				continue;
			};
			if only_initial {
				info!("initial sync {} -> {target}", entry.abs_path.display());
			} else {
				info!("resync {} -> {target}", entry.abs_path.display());
			}
			if let Err(e) = self
				.sync_to_container(
					&entry.container_name,
					&entry.abs_path,
					&entry.abs_path,
					target,
					ensured,
				)
				.await
			{
				warn!(
					"{} failed: {e}",
					if only_initial {
						"initial sync"
					} else {
						"resync"
					}
				);
			}
		}
	}
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
