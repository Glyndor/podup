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
use crate::compose::types::WatchAction;
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
/// downstream, plus a `Rescan` event (the inotify queue overflow signal):
/// that one has no path the host cares about and the loop's
/// `is_dispatch_event` check would otherwise discard it before the loop
/// wakes up, so the recovery path would never know the kernel itself
/// declared an overflow. The flag and the channel both carry it; the
/// loop's filter still drops it after the recovery runs.
pub(super) fn should_enqueue(res: &WatchEvent) -> bool {
	match res {
		Err(_) => true,
		Ok(e) => e.need_rescan() || is_dispatch_event(&e.kind),
	}
}

/// Push `res` into the watch channel, or record an overflow.
///
/// A `Rescan` event (the inotify queue overflow signal) sets the flag
/// before any `try_send`: that path has nothing to deliver to the loop's
/// dispatch logic, but the loop still has to see the kernel's "you missed
/// something" so the recovery path runs. Sending it on is best-effort:
/// the loop's `is_dispatch_event` filter drops it again after the
/// recovery has had a chance to look at the flag.
///
/// On `TrySendError::Full` the overflow flag is set so the loop resyncs
/// on its next iteration, and the existing `debug!` line keeps the drop
/// observable to operators. `Closed` is the normal "the consumer went
/// away" path and gets only the same `debug!` line.
pub(super) fn enqueue(
	tx: &mpsc::Sender<WatchEvent>,
	overflow: &std::sync::atomic::AtomicBool,
	res: WatchEvent,
) {
	let needs_rescan = match &res {
		Ok(e) => e.need_rescan(),
		Err(_) => false,
	};
	if !should_enqueue(&res) {
		return;
	}
	if needs_rescan {
		overflow.store(true, std::sync::atomic::Ordering::SeqCst);
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
/// A `NotFound` or `NotADirectory` error means the host path is absent (the
/// file was deleted, or the rule's root is a file and the changed entry is
/// under it), so a `Remove` is the right call. Any other error (a permission
/// error, an EIO on a network mount, a stale handle) is propagated to the
/// caller: the path on disk is unknown, and removing the container copy
/// because the host was briefly unreadable is exactly the bug this function
/// exists to prevent.
pub(super) fn sync_op_for(path: &Path) -> std::io::Result<SyncOp> {
	match std::fs::symlink_metadata(path) {
		Ok(_) => Ok(SyncOp::Upload),
		Err(e) => match e.kind() {
			std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => Ok(SyncOp::Remove),
			_ => Err(e),
		},
	}
}

impl Engine {
	/// Run the per-rule sync loop. Used both at startup and on an
	/// overflow-driven resync; both paths run only the rules with
	/// `initial_sync: true`, because that flag is the user's contract
	/// for "this rule accepts a full upload". A rule without
	/// `initial_sync` filters what it accepts through its `ignore` /
	/// `include` lists; a recovery-driven full upload would ignore those
	/// filters and could write a file the rule was configured to
	/// exclude. Rules whose sync step is `sync_redundant` (target is
	/// already shared through a bind mount the rule also targets) are
	/// skipped here as well: the restart / exec part of the action
	/// still runs, but the sync is by definition a no-op. `label` is
	/// what the `info!` / `warn!` lines print (`"initial sync"` at
	/// startup, `"resync"` from the overflow recovery).
	pub(super) async fn sync_all(
		&self,
		rule_entries: &[RuleEntry],
		ensured: &mut HashSet<(String, String)>,
		label: &str,
	) {
		for entry in rule_entries {
			if !entry.rule.initial_sync {
				continue;
			}
			if entry.sync_redundant {
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
			info!("{label} {} -> {target}", entry.abs_path.display());
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
				warn!("{label} failed: {e}");
			}
		}
	}

	/// The rules the overflow path could not cover. A plain `sync` rule
	/// with `initial_sync` IS covered (the resync uploads it whole).
	/// Anything else (a sync rule without `initial_sync`, any
	/// `sync+restart` / `sync+exec` rule whose restart or exec was not
	/// re-run, any `rebuild` / `restart` rule) is named in the warning
	/// the operator reads to decide whether to restart `podup watch`.
	pub(super) fn rules_not_recovered(rule_entries: &[RuleEntry]) -> Vec<String> {
		let tuples: Vec<(String, String, WatchAction, bool)> = rule_entries
			.iter()
			.map(|e| {
				(
					e.service_name.clone(),
					e.rule.path.clone(),
					e.rule.action.clone(),
					e.rule.initial_sync,
				)
			})
			.collect();
		Self::rules_not_recovered_tuples(&tuples)
	}

	/// Tuple form of [`Self::rules_not_recovered`] for unit tests; the
	/// real entry struct's fields live in the parent module and are not
	/// reachable from `events_tests.rs` directly.
	pub(super) fn rules_not_recovered_tuples(
		rules: &[(String, String, WatchAction, bool)],
	) -> Vec<String> {
		let mut out = Vec::new();
		for (service, path, action, initial_sync) in rules {
			if *initial_sync && matches!(action, WatchAction::Sync) {
				continue;
			}
			out.push(format!("{service}:{path}"));
		}
		out
	}

	/// When the watch channel has signalled an overflow (either the
	/// bounded queue dropped an event or notify itself reported
	/// `IN_Q_OVERFLOW` as a `Rescan` event), re-run the same sync the
	/// initial startup ran, name the rules that the recovery could not
	/// cover, and clear the flag. Called from the loop at the top of the
	/// iteration and again after a batch has been dispatched, so a
	/// queue overflow that surfaced while a batch was being processed
	/// still gets the recovery before the next batch.
	pub(super) async fn recover_from_overflow(
		&self,
		overflow: &std::sync::atomic::AtomicBool,
		rule_entries: &[RuleEntry],
		ensured: &mut HashSet<(String, String)>,
	) {
		if !overflow.swap(false, std::sync::atomic::Ordering::SeqCst) {
			return;
		}
		self.sync_all(rule_entries, ensured, "resync").await;
		let unrecovered = Self::rules_not_recovered(rule_entries);
		if unrecovered.is_empty() {
			warn!("watch event queue overflowed; resynced every rule");
		} else {
			warn!(
				"watch event queue overflowed; resynced the rules with initial_sync, but changes for {list} may not have been applied; restart podup watch to apply them",
				list = unrecovered.join(", ")
			);
		}
	}
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
